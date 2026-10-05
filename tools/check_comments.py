#!/usr/bin/env python3
"""Enforce the two-line comment-block budget of docs/CONVENTIONS.md.
Covers .rs/.py here and C/C++ changed versus upstream/master in the ScummVM fork."""
import argparse
import ast
import io
import re
import subprocess
import sys
import tokenize
from dataclasses import dataclass
from pathlib import Path

MAX_BLOCK_LINES = 2
REPO_ROOT = Path(__file__).resolve().parent.parent
FORK_DIR = REPO_ROOT / "scummvm"
FORK_BASE_REF = "upstream/master"
SKIPPED_DIRS = {"scummvm", ".venv", "work", "media", "target", ".git", "__pycache__", "node_modules"}
OWN_SUFFIXES = {".rs", ".py"}
FORK_SUFFIXES = {".c", ".cc", ".cpp", ".h", ".hpp"}
LICENSE_OPENER = "/* ScummVM"
ENGINES_PATH = re.compile(r"(^|/)engines/")


@dataclass(frozen=True)
class Comment:
    first_line: int
    last_line: int
    column: int
    standalone: bool
    is_doxygen: bool = False


@dataclass(frozen=True)
class Violation:
    path: str
    line: int
    length: int

    def __str__(self):
        return f"{self.path}:{self.line}: comment block of {self.length} lines (max {MAX_BLOCK_LINES})"


def scan_c_like_comments(text, nested_blocks, raw_strings):
    """Return every comment in C, C++ or Rust source, with string and char literals skipped."""
    comments = []
    line, line_start = 1, 0
    code_on_line = False
    i, n = 0, len(text)
    while i < n:
        ch = text[i]
        pair = text[i : i + 2]
        if ch == "\n":
            line, line_start, code_on_line = line + 1, i + 1, False
            i += 1
        elif pair == "//":
            end = text.find("\n", i)
            end = n if end < 0 else end
            doxygen = text[i : i + 3] in ("///", "//!")
            comments.append(Comment(line, line, i - line_start, not code_on_line, doxygen))
            i = end
        elif pair == "/*":
            end, depth = i + 2, 1
            while end < n and depth:
                if nested_blocks and text[end : end + 2] == "/*":
                    depth, end = depth + 1, end + 2
                elif text[end : end + 2] == "*/":
                    depth, end = depth - 1, end + 2
                else:
                    end += 1
            body = text[i:end]
            span = body.count("\n")
            doxygen = body.startswith(("/**", "/*!")) and not body.startswith("/**/")
            comments.append(Comment(line, line + span, i - line_start, not code_on_line, doxygen))
            if span:
                line, line_start, code_on_line = line + span, i + body.rfind("\n") + 1, True
            i = end
        elif ch == '"' or (raw_strings and ch == "r" and re.match(r'r#*"', text[i:])):
            end = skip_string(text, i, raw_strings)
            line, line_start = advance_lines(text, i, end, line, line_start)
            code_on_line = True
            i = end
        elif ch == "'":
            end = skip_char_literal(text, i)
            code_on_line = True
            i = end
        else:
            code_on_line = code_on_line or not ch.isspace()
            i += 1
    return comments


def skip_string(text, start, raw_strings):
    raw = re.match(r'r(#*)"', text[start:]) if raw_strings and text[start] == "r" else None
    if raw:
        closing = '"' + raw.group(1)
        end = text.find(closing, start + len(raw.group(0)))
        return len(text) if end < 0 else end + len(closing)
    i = start + 1
    while i < len(text) and text[i] != '"':
        i += 2 if text[i] == "\\" else 1
    return min(i + 1, len(text))


def skip_char_literal(text, start):
    match = re.match(r"'(\\x[0-9a-fA-F]{2}|\\u\{[0-9a-fA-F]+\}|\\.|[^\\'\n])'", text[start:])
    return start + match.end() if match else start + 1


def advance_lines(text, start, end, line, line_start):
    newlines = text.count("\n", start, end)
    if newlines:
        line_start = text.rfind("\n", start, end) + 1
    return line + newlines, line_start


def scan_python_comments(text):
    """Return every comment and every multi-line docstring in Python source."""
    comments = []
    code_lines = set()
    for token in tokenize.generate_tokens(io.StringIO(text).readline):
        row, column = token.start
        if token.type == tokenize.COMMENT:
            if not (row == 1 and token.string.startswith("#!")):
                comments.append(Comment(row, row, column, row not in code_lines))
        elif token.type not in (tokenize.NL, tokenize.NEWLINE, tokenize.INDENT, tokenize.DEDENT):
            code_lines.add(row)
    return comments + python_docstrings(text)


def python_docstrings(text):
    docstrings = []
    for node in ast.walk(ast.parse(text)):
        if isinstance(node, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            first = node.body[0] if node.body else None
            if isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant):
                if isinstance(first.value.value, str):
                    docstrings.append(Comment(first.lineno, first.end_lineno, first.col_offset, True))
    return docstrings


def group_into_blocks(comments):
    """Merge comments on consecutive lines into blocks; a trailing comment only joins its own column."""
    blocks = []
    for comment in sorted(comments, key=lambda c: (c.first_line, c.column)):
        if blocks and continues_block(blocks[-1], comment):
            previous = blocks[-1]
            blocks[-1] = Comment(previous.first_line, comment.last_line, previous.column,
                                 previous.standalone, previous.is_doxygen and comment.is_doxygen)
        else:
            blocks.append(comment)
    return blocks


def continues_block(block, comment):
    adjacent = comment.first_line == block.last_line + 1
    if not (adjacent and comment.standalone):
        return False
    return block.standalone or comment.column == block.column


def is_exempt(block, text, path):
    opens_file = block.first_line == 1 and text.startswith(LICENSE_OPENER)
    outside_engines = block.is_doxygen and not ENGINES_PATH.search(path)
    return opens_file or outside_engines


def find_violations(path, text, changed_lines=None):
    """Blocks over budget in one file; with changed_lines, only blocks touching those lines."""
    if path.endswith(".py"):
        comments = scan_python_comments(text)
    else:
        comments = scan_c_like_comments(text, nested_blocks=path.endswith(".rs"),
                                        raw_strings=path.endswith(".rs"))
    violations = []
    for block in group_into_blocks(comments):
        length = block.last_line - block.first_line + 1
        if length <= MAX_BLOCK_LINES or is_exempt(block, text, path):
            continue
        touched = changed_lines is None or any(
            block.first_line <= number <= block.last_line for number in changed_lines)
        if touched:
            violations.append(Violation(path, block.first_line, length))
    return violations


def run_git(repo, *args):
    result = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    return result.stdout if result.returncode == 0 else None


def parse_added_lines(diff_text):
    """Map each file in a zero-context diff to the set of line numbers it adds."""
    added, current = {}, None
    for row in diff_text.splitlines():
        if row.startswith("+++ b/"):
            current = added.setdefault(row[6:], set())
        elif row.startswith("@@") and current is not None:
            match = re.search(r"\+(\d+)(?:,(\d+))?", row)
            start, count = int(match.group(1)), int(match.group(2) or 1)
            current.update(range(start, start + count))
    return added


def untracked_files(repo):
    listing = run_git(repo, "ls-files", "--others", "--exclude-standard")
    return listing.split() if listing else []


def fork_changes(diff_args):
    """Added line numbers per C/C++ file in the fork; untracked files count in full."""
    diff_text = run_git(FORK_DIR, "diff", "-U0", "--diff-filter=ACMR", *diff_args)
    if diff_text is None:
        return None
    changes = parse_added_lines(diff_text)
    for name in untracked_files(FORK_DIR):
        changes[name] = None
    return {name: lines for name, lines in changes.items() if Path(name).suffix in FORK_SUFFIXES}


def check_fork_files(changes):
    violations = []
    for name, lines in sorted(changes.items()):
        text = (FORK_DIR / name).read_text(encoding="utf-8", errors="replace")
        violations += find_violations(f"scummvm/{name}", text, lines)
    return violations


def own_files_in_tree():
    for path in sorted(REPO_ROOT.rglob("*")):
        relative = path.relative_to(REPO_ROOT)
        if path.is_file() and path.suffix in OWN_SUFFIXES and not SKIPPED_DIRS & set(relative.parts):
            yield path


def own_files_in_range(revision_range):
    names = run_git(REPO_ROOT, "diff", "--name-only", "--diff-filter=ACMR", revision_range)
    paths = [REPO_ROOT / name for name in (names or "").split()]
    return [p for p in paths if p.suffix in OWN_SUFFIXES and not SKIPPED_DIRS & set(p.relative_to(REPO_ROOT).parts)]


def check_whole_files(paths):
    violations = []
    for path in paths:
        text = path.read_text(encoding="utf-8", errors="replace")
        relative = str(path.relative_to(REPO_ROOT)) if path.is_relative_to(REPO_ROOT) else str(path)
        violations += find_violations(relative, text)
    return violations


def collect_violations(target):
    if target is None:
        changes = fork_changes([FORK_BASE_REF]) if FORK_DIR.exists() else {}
        return check_whole_files(own_files_in_tree()) + check_fork_files(changes or {})
    path = Path(target)
    if path.exists():
        files = [path] if path.is_file() else sorted(p for p in path.rglob("*") if p.is_file())
        return check_whole_files([f for f in files if f.suffix in OWN_SUFFIXES | FORK_SUFFIXES])
    violations = check_whole_files(own_files_in_range(target))
    changes = fork_changes([target]) if FORK_DIR.exists() else None
    return violations + (check_fork_files(changes) if changes is not None else [])


def self_test():
    cases = [
        ("a.rs", "fn a() {}\n// one\n// two\nfn b() {}\n", 0),
        ("a.rs", "fn a() {}\n// one\n// two\n// three\nfn b() {}\n", 1),
        ("a.rs", 'let s = "// not a comment\n// nor this\n// nor this";\n', 0),
        ("a.rs", "let c = 'a'; let l: &'static str = x; // one\n", 0),
        ("a.rs", "/* a\n b\n c */\n", 1),
        ("a.rs", "/* a /* nested\n b */\n c */\n", 1),
        ("a.rs", "let a = 1; // one\n           // two\n           // three\n", 1),
        ("a.rs", "let a = 1; // one\nlet b = 2; // two\nlet c = 3; // three\n", 0),
        ("a.py", "x = 1\n# one\n# two\n# three\n", 1),
        ("a.py", "#!/usr/bin/env python3\n# one\n# two\nx = 1\n", 0),
        ("a.py", '"""one\ntwo\nthree"""\n', 1),
        ("a.py", 'x = """a\n# b\n# c\n# d"""\n', 0),
        ("a.cpp", "/* ScummVM - x\n *\n * licence\n */\nint a;\n", 0),
        ("engines/sci/a.cpp", "/**\n * doc\n * doc\n */\nint a;\n", 1),
        ("common/a.h", "/**\n * doc\n * doc\n */\nint a;\n", 0),
    ]
    failures = 0
    for path, source, expected in cases:
        found = len(find_violations(path, source))
        if found != expected:
            failures += 1
            print(f"self-test FAIL {path!r} {source!r}: expected {expected}, got {found}")
    print("self-test: " + ("all passed" if not failures else f"{failures} failed"))
    return 1 if failures else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", nargs="?",
                        help="file, directory or git range (default: whole repo and fork vs upstream/master)")
    parser.add_argument("--self-test", action="store_true", help="prove the scanner can fail")
    arguments = parser.parse_args()
    if arguments.self_test:
        return self_test()
    violations = collect_violations(arguments.target)
    for violation in violations:
        print(violation)
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
