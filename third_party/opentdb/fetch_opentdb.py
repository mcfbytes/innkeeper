#!/usr/bin/env python3
"""Download a snapshot of the Open Trivia Database.

The database is published at https://opentdb.com under CC BY-SA 4.0.
This script uses only the Python 3 standard library.

What it does
------------
1. Asks the API for a session token. A token makes the server skip
   questions it has already sent, so paging through a category does not
   return duplicates.
2. Reads the category list, then the verified-question count for each
   category (https://opentdb.com/api_count.php?category=N).
3. Pulls that category with amount=50 (the API maximum), the session
   token, and encode=url3986, until the token is exhausted for the
   category (response_code 4).
4. Decodes the percent-encoded fields, drops duplicate
   category+question pairs, and writes JSON sorted by a stable id.

The API allows one request per 5 seconds per IP. This script waits at
least 5.5 seconds between requests. Response code 5 (rate limit) is
not treated as data: the script waits and retries the same call.

Why the last page is sometimes smaller than 50
-----------------------------------------------
Requesting more questions than the token can still return yields
response_code 4 and an empty result list, and does not consume the
questions that remain. A category with 40 questions therefore answers
amount=50 with code 4, then answers amount=40 normally. The count
endpoint says how many to ask for, so a code 4 that arrives after a
correctly sized request really does mean the category is exhausted.
The script still issues one more request after the counted questions
and requires response_code 4 (or code 1 on a one-question probe)
before it moves on.

Usage
-----
    python3 fetch_opentdb.py opentdb.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

API_ROOT = "https://opentdb.com"
SOURCE = "https://opentdb.com"
LICENSE_NAME = "CC BY-SA 4.0"

# One request per 5 seconds is the published limit. Stay a little over it.
MIN_INTERVAL_S = 5.5

# Response codes from https://opentdb.com/api_config.php
CODE_SUCCESS = 0
CODE_NO_RESULTS = 1
CODE_INVALID = 2
CODE_TOKEN_NOT_FOUND = 3
CODE_TOKEN_EMPTY = 4
CODE_RATE_LIMIT = 5

# How many times to repeat one call that was rate-limited or dropped.
MAX_ATTEMPTS = 8

USER_AGENT = "opentdb-snapshot/1.0 (Python urllib; CC BY-SA 4.0 archive)"

QUESTION_FIELDS = (
    "category",
    "type",
    "difficulty",
    "question",
    "correct_answer",
    "incorrect_answers",
)


class ApiError(RuntimeError):
    """The API returned a response this script will not retry."""


class OpentdbClient:
    """HTTP client that spaces calls out and retries transport failures.

    The clock starts when a response finishes, and the next request waits
    until MIN_INTERVAL_S has passed. That gap is the "between requests"
    delay, independent of how long each response took to arrive.
    """

    def __init__(self) -> None:
        self._last_finished = 0.0
        self.requests = 0

    def get_json(self, url: str) -> dict:
        """GET a URL and parse the body as JSON. Waits between calls."""
        delay = MIN_INTERVAL_S
        last_error: Exception | None = None
        for attempt in range(1, MAX_ATTEMPTS + 1):
            self._wait_for_slot()
            request = urllib.request.Request(
                url,
                headers={
                    "User-Agent": USER_AGENT,
                    "Accept": "application/json",
                },
            )
            try:
                with urllib.request.urlopen(request, timeout=60) as response:
                    body = response.read()
                self._last_finished = time.monotonic()
                self.requests += 1
            except urllib.error.HTTPError as exc:
                # A finished HTTP error still consumed the rate-limit slot.
                self._last_finished = time.monotonic()
                self.requests += 1
                last_error = exc
                if exc.code == 429 or exc.code >= 500:
                    print(
                        f"  HTTP {exc.code} on attempt {attempt}; "
                        f"waiting {delay:.1f}s",
                        flush=True,
                    )
                    time.sleep(delay)
                    delay = min(delay * 2, 60.0)
                    continue
                raise ApiError(f"HTTP {exc.code} for {url}") from exc
            except (urllib.error.URLError, TimeoutError, OSError) as exc:
                self._last_finished = time.monotonic()
                last_error = exc
                print(
                    f"  network error on attempt {attempt} ({exc}); "
                    f"waiting {delay:.1f}s",
                    flush=True,
                )
                time.sleep(delay)
                delay = min(delay * 2, 60.0)
                continue

            try:
                data = json.loads(body.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                last_error = exc
                print(
                    f"  unreadable JSON on attempt {attempt}; "
                    f"waiting {delay:.1f}s",
                    flush=True,
                )
                time.sleep(delay)
                delay = min(delay * 2, 60.0)
                continue
            if not isinstance(data, dict):
                raise ApiError(f"expected a JSON object from {url}")
            return data

        raise ApiError(f"giving up on {url} after {MAX_ATTEMPTS} attempts: {last_error}")

    def _wait_for_slot(self) -> None:
        if self._last_finished == 0.0:
            return
        remaining = MIN_INTERVAL_S - (time.monotonic() - self._last_finished)
        if remaining > 0:
            time.sleep(remaining)


def decode_url3986(value):
    """Decode an RFC 3986 percent-encoded API value.

    encode=url3986 uses %20 for a space, so this is urllib.parse.unquote
    and not unquote_plus (which would turn a literal '+' into a space).
    Lists are decoded element by element. The question text is otherwise
    left as the API sent it.
    """
    if isinstance(value, str):
        return urllib.parse.unquote(value)
    if isinstance(value, list):
        return [decode_url3986(item) for item in value]
    return value


def question_id(category: str, question: str) -> str:
    """Stable id: SHA-1 hex of the UTF-8 bytes of category then question.

    The two strings are concatenated with no separator. The id names the
    question text inside its category; it is not a security hash.
    """
    digest = hashlib.sha1((category + question).encode("utf-8"))
    return digest.hexdigest()


def normalize_question(raw: dict) -> dict:
    """Turn one API result into the snapshot record. Raises ApiError if incomplete."""
    missing = [field for field in QUESTION_FIELDS if field not in raw]
    if missing:
        raise ApiError(f"question is missing {missing}")
    category = decode_url3986(raw["category"])
    question = decode_url3986(raw["question"])
    incorrect = decode_url3986(raw["incorrect_answers"])
    if not isinstance(category, str) or not isinstance(question, str):
        raise ApiError("category and question must be strings")
    if not isinstance(incorrect, list) or not all(isinstance(item, str) for item in incorrect):
        raise ApiError("incorrect_answers must be a list of strings")
    return {
        "id": question_id(category, question),
        "category": category,
        "type": decode_url3986(raw["type"]),
        "difficulty": decode_url3986(raw["difficulty"]),
        "question": question,
        "correct_answer": decode_url3986(raw["correct_answer"]),
        "incorrect_answers": incorrect,
    }


def request_token(client: OpentdbClient) -> str:
    data = client.get_json(f"{API_ROOT}/api_token.php?command=request")
    code = data.get("response_code")
    token = data.get("token")
    if code != CODE_SUCCESS or not isinstance(token, str) or not token:
        raise ApiError(f"could not retrieve a session token: {data}")
    return token


def request_categories(client: OpentdbClient) -> list[dict]:
    data = client.get_json(f"{API_ROOT}/api_category.php")
    raw = data.get("trivia_categories")
    if not isinstance(raw, list) or not raw:
        raise ApiError(f"category list was empty: {data}")
    categories = []
    for item in raw:
        if not isinstance(item, dict) or "id" not in item or "name" not in item:
            raise ApiError(f"unexpected category entry: {item!r}")
        categories.append({"id": int(item["id"]), "name": str(item["name"])})
    categories.sort(key=lambda category: category["id"])
    return categories


def request_count(client: OpentdbClient, category_id: int) -> int:
    data = client.get_json(f"{API_ROOT}/api_count.php?category={category_id}")
    counts = data.get("category_question_count") or {}
    total = counts.get("total_question_count")
    if not isinstance(total, int) or total < 0:
        raise ApiError(f"no question count for category {category_id}: {data}")
    return total


def request_questions(client: OpentdbClient, token: str, category_id: int, amount: int):
    """Return (response_code, results). Response code 5 is waited out."""
    # amount, category, token, and encode are the parameters the snapshot uses.
    # amount is 50 until fewer than 50 questions remain; see the module docstring.
    query = urllib.parse.urlencode(
        {
            "amount": amount,
            "category": category_id,
            "token": token,
            "encode": "url3986",
        }
    )
    url = f"{API_ROOT}/api.php?{query}"
    delay = MIN_INTERVAL_S
    for attempt in range(1, MAX_ATTEMPTS + 1):
        data = client.get_json(url)
        code = data.get("response_code")
        if code == CODE_RATE_LIMIT:
            print(
                f"  rate limited (response_code 5), attempt {attempt}; "
                f"waiting {delay:.1f}s",
                flush=True,
            )
            time.sleep(delay)
            delay = min(delay * 2, 60.0)
            continue
        results = data.get("results") or []
        if not isinstance(results, list):
            raise ApiError(f"results was not a list for category {category_id}")
        return code, results
    raise ApiError(f"category {category_id} stayed rate-limited")


def add_results(records: dict, results: list, category_name: str, warnings: list) -> int:
    """Store decoded questions. Return how many new ids were added.

    The same category+question id keeps the first copy. A later copy with
    different answers is kept out of the snapshot and listed in warnings.
    """
    added = 0
    for raw in results:
        record = normalize_question(raw)
        if record["category"] != category_name:
            message = (
                f"category label {record['category']!r} while fetching {category_name!r}"
            )
            if message not in warnings:
                warnings.append(message)
        previous = records.get(record["id"])
        if previous is None:
            records[record["id"]] = record
            added += 1
            continue
        if previous != record:
            warnings.append(
                f"duplicate id {record['id']} in {category_name!r} "
                f"with different answers; kept the first copy"
            )
    return added


def collect_category(client, token: str, category: dict, expected: int, warnings: list) -> dict:
    """Pull every question the token will give for one category.

    Returns the records keyed by id. Stops only after a one-question
    request comes back empty (response_code 4, or code 1), which is the
    API's exhaustion signal once `amount` is known not to overshoot.
    """
    records: dict[str, dict] = {}
    name = category["name"]
    category_id = category["id"]
    # A healthy category is one page per 50 questions, plus a confirmation
    # request, plus a few retries if the published count is off.
    max_calls = max(8, (expected // 50) + 40)
    calls = 0

    def fetch(amount: int):
        nonlocal calls
        calls += 1
        if calls > max_calls:
            raise ApiError(
                f"category {category_id} ({name}) exceeded {max_calls} calls"
            )
        return request_questions(client, token, category_id, amount)

    # Phase 1: take the published count, a page of 50 at a time.
    # If the count is too high, a refused page lowers the remainder and
    # the same loop tries a smaller amount. Questions are not consumed
    # by a refused page.
    remaining = expected
    while remaining > 0:
        amount = min(50, remaining)
        code, results = fetch(amount)
        if code == CODE_TOKEN_NOT_FOUND:
            raise ApiError("session token was rejected (response_code 3)")
        if code == CODE_INVALID:
            raise ApiError(f"category {category_id} rejected the query (response_code 2)")
        if code == CODE_SUCCESS and results:
            added = add_results(records, results, name, warnings)
            print(
                f"  received {len(results)} (amount={amount}), "
                f"{len(records)} unique so far, expected {expected}",
                flush=True,
            )
            if added == 0:
                raise ApiError(
                    f"category {category_id} returned only questions already stored"
                )
            remaining -= len(results)
            if len(results) < amount:
                # A short page means the server had nothing further for this query.
                remaining = 0
            continue
        if code in (CODE_NO_RESULTS, CODE_TOKEN_EMPTY) or (code == CODE_SUCCESS and not results):
            if amount == 1:
                print(
                    f"  response_code {code} for the last question; category exhausted",
                    flush=True,
                )
                return records
            print(
                f"  response_code {code} for amount={amount} with "
                f"{remaining} still expected; retrying a smaller page",
                flush=True,
            )
            remaining = amount - 1
            continue
        raise ApiError(f"category {category_id} returned response_code {code}")

    # Phase 2: the count is in hand (or was zero). Ask until the token
    # reports that this category is empty. amount starts at 1 so an
    # accurate count costs a single response_code 4. If that call still
    # returns a question, the count was low and paging resumes at 50.
    amount = 1
    while True:
        code, results = fetch(amount)
        if code == CODE_TOKEN_NOT_FOUND:
            raise ApiError("session token was rejected (response_code 3)")
        if code == CODE_INVALID:
            raise ApiError(f"category {category_id} rejected the query (response_code 2)")
        if code == CODE_SUCCESS and results:
            added = add_results(records, results, name, warnings)
            print(
                f"  count was short; received {len(results)} more "
                f"({len(records)} unique, expected {expected})",
                flush=True,
            )
            if added == 0:
                raise ApiError(
                    f"category {category_id} returned only questions already stored"
                )
            # A full page may have another full page behind it. A short
            # page should be followed by an exhaustion check of amount=1.
            amount = 50 if len(results) == amount else 1
            continue
        if code in (CODE_NO_RESULTS, CODE_TOKEN_EMPTY) or (code == CODE_SUCCESS and not results):
            if amount == 1:
                print(
                    f"  token exhausted (response_code {code}), "
                    f"{len(records)} unique, count said {expected}",
                    flush=True,
                )
                return records
            # amount was larger than what remains. Halve and try again.
            # This refusal does not consume the leftover questions.
            nxt = max(1, amount // 2)
            print(
                f"  response_code {code} for amount={amount}; retrying amount={nxt}",
                flush=True,
            )
            amount = nxt
            continue
        raise ApiError(f"category {category_id} returned response_code {code}")


def write_snapshot(path: str, categories: list[dict], questions: list[dict], fetched: str) -> None:
    payload = {
        "source": SOURCE,
        "license": LICENSE_NAME,
        "fetched": fetched,
        "categories": categories,
        "questions": questions,
    }
    temporary = path + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    # Replace in one step so a reader never sees a half-written file.
    os.replace(temporary, path)


def verify_snapshot(questions: list[dict]) -> None:
    """Check the invariants the output promises. Raises ApiError on a bug."""
    ids = [question["id"] for question in questions]
    if ids != sorted(ids):
        raise ApiError("questions were not sorted by id")
    if len(ids) != len(set(ids)):
        raise ApiError("question ids are not unique")
    for question in questions:
        expected = question_id(question["category"], question["question"])
        if question["id"] != expected:
            raise ApiError(f"id does not match category+question: {question['id']}")
        for field in ("id",) + QUESTION_FIELDS:
            if field not in question:
                raise ApiError(f"question missing {field}")


def print_summary(questions: list[dict], categories: list[dict], expected: dict[int, int], warnings: list) -> None:
    by_category: dict[str, int] = {}
    by_type: dict[str, int] = {}
    by_difficulty: dict[str, int] = {}
    for question in questions:
        by_category[question["category"]] = by_category.get(question["category"], 0) + 1
        by_type[question["type"]] = by_type.get(question["type"], 0) + 1
        by_difficulty[question["difficulty"]] = by_difficulty.get(question["difficulty"], 0) + 1

    print(f"total questions: {len(questions)}", flush=True)
    print("per category:", flush=True)
    for category in categories:
        got = by_category.get(category["name"], 0)
        print(
            f"  {category['id']:>2}  {category['name']}: {got} "
            f"(count endpoint {expected.get(category['id'], '?')})",
            flush=True,
        )
    print("per type:", flush=True)
    for name in sorted(by_type):
        print(f"  {name}: {by_type[name]}", flush=True)
    print("per difficulty:", flush=True)
    for name in sorted(by_difficulty):
        print(f"  {name}: {by_difficulty[name]}", flush=True)
    if warnings:
        print(f"warnings ({len(warnings)}):", flush=True)
        for warning in warnings:
            print(f"  - {warning}", flush=True)
    else:
        print("warnings: none", flush=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Download an Open Trivia Database snapshot to a JSON file."
    )
    parser.add_argument("output", help="path of the JSON snapshot to write")
    args = parser.parse_args(argv)

    client = OpentdbClient()
    warnings: list[str] = []
    started = time.monotonic()

    print("requesting session token", flush=True)
    token = request_token(client)
    print("requesting categories", flush=True)
    categories = request_categories(client)
    print(f"{len(categories)} categories", flush=True)

    expected: dict[int, int] = {}
    records: dict[str, dict] = {}
    for index, category in enumerate(categories, start=1):
        total = request_count(client, category["id"])
        expected[category["id"]] = total
        print(
            f"[{index}/{len(categories)}] {category['name']} "
            f"(id {category['id']}): count {total}",
            flush=True,
        )
        collected = collect_category(client, token, category, total, warnings)
        # Ids include the category text, so categories should not collide.
        overlap = set(records).intersection(collected)
        if overlap:
            warnings.append(
                f"{len(overlap)} ids from {category['name']!r} were already stored"
            )
        records.update(collected)
        if len(collected) != total:
            warnings.append(
                f"{category['name']}: stored {len(collected)} unique questions, "
                f"count endpoint said {total}"
            )

    questions = sorted(records.values(), key=lambda question: question["id"])
    verify_snapshot(questions)
    fetched = datetime.now(timezone.utc).date().isoformat()
    write_snapshot(args.output, categories, questions, fetched)

    elapsed = time.monotonic() - started
    print(
        f"wrote {args.output} ({len(questions)} questions, "
        f"{client.requests} HTTP requests, {elapsed / 60:.1f} min)",
        flush=True,
    )
    print_summary(questions, categories, expected, warnings)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ApiError as exc:
        print(f"error: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
