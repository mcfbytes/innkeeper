use std::iter::Peekable;
use std::str::Chars;

/// One command from an `AT` line, as far as the impersonation needs to tell them apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AtCommand {
    Dial(String),
    Online,
    HangUp,
    Reset,
    Echo(bool),
    /// `\B`: the NOBRK.DRV escape asks the modem to send the BREAK it cannot send itself.
    SendBreak,
    Other(String),
}

/// Splits a command line into commands; `None` when it holds no `AT`, which a modem ignores.
pub(crate) fn parse_command_line(line: &str) -> Option<Vec<AtCommand>> {
    let upper = line.to_ascii_uppercase();
    let after_prefix = upper.get(upper.find("AT")? + 2..)?;
    let mut commands = Vec::new();
    let mut chars = after_prefix.chars().peekable();
    while let Some(letter) = chars.next() {
        let command = match letter {
            ' ' => continue,
            'D' => {
                commands.push(AtCommand::Dial(dial_number(chars.collect())));
                break;
            }
            'E' => AtCommand::Echo(take_number(&mut chars).unwrap_or(0) != 0),
            'H' => match take_number(&mut chars) {
                None | Some(0) => AtCommand::HangUp,
                Some(n) => AtCommand::Other(format!("H{n}")),
            },
            'O' => skip_number_then(&mut chars, AtCommand::Online),
            'Z' => skip_number_then(&mut chars, AtCommand::Reset),
            '\\' if chars.peek() == Some(&'B') => {
                chars.next();
                skip_number_then(&mut chars, AtCommand::SendBreak)
            }
            '&' | '%' | '\\' | '+' | '#' | '*' | '-' => {
                let name: String = [Some(letter), chars.next()].into_iter().flatten().collect();
                AtCommand::Other(format!("{name}{}", take_digits(&mut chars)))
            }
            'S' => AtCommand::Other(format!("S{}", take_register_access(&mut chars))),
            other => AtCommand::Other(format!("{other}{}", take_digits(&mut chars))),
        };
        commands.push(command);
    }
    Some(commands)
}

fn dial_number(rest: String) -> String {
    let trimmed = rest.trim();
    let without_mode = trimmed.strip_prefix(['T', 'P']).unwrap_or(trimmed);
    without_mode.trim().to_owned()
}

fn take_digits(chars: &mut Peekable<Chars<'_>>) -> String {
    std::iter::from_fn(|| chars.next_if(char::is_ascii_digit)).collect()
}

fn take_number(chars: &mut Peekable<Chars<'_>>) -> Option<u32> {
    take_digits(chars).parse().ok()
}

fn skip_number_then(chars: &mut Peekable<Chars<'_>>, command: AtCommand) -> AtCommand {
    take_digits(chars);
    command
}

/// `Sn=v` writes a register and `Sn?` reads one.
fn take_register_access(chars: &mut Peekable<Chars<'_>>) -> String {
    let register = take_digits(chars);
    match chars.next_if(|&c| c == '=' || c == '?') {
        Some('=') => format!("{register}={}", take_digits(chars)),
        Some(query) => format!("{register}{query}"),
        None => register,
    }
}

#[cfg(test)]
mod tests {
    use super::AtCommand::*;
    use super::*;

    fn parse(line: &str) -> Vec<AtCommand> {
        parse_command_line(line).unwrap()
    }

    #[test]
    fn lines_without_at_are_ignored() {
        assert_eq!(parse_command_line("+++"), None);
        assert_eq!(parse_command_line(""), None);
    }

    #[test]
    fn dial_takes_the_rest_of_the_line() {
        assert_eq!(parse("ATDT5551234"), vec![Dial("5551234".into())]);
        assert_eq!(parse("atdp 555-1234 E0"), vec![Dial("555-1234 E0".into())]);
        assert_eq!(
            parse("+++ATDT127.0.0.1:2314"),
            vec![Dial("127.0.0.1:2314".into())]
        );
    }

    #[test]
    fn init_strings_from_modem_txt_parse() {
        assert_eq!(parse("ATZ"), vec![Reset]);
        assert_eq!(parse("AT&D2"), vec![Other("&D2".into())]);
        assert_eq!(
            parse("AT E1 V1 S0=0"),
            vec![Echo(true), Other("V1".into()), Other("S0=0".into())]
        );
        assert_eq!(parse("ATE0"), vec![Echo(false)]);
        assert_eq!(parse("ATE"), vec![Echo(false)]);
    }

    #[test]
    fn escape_and_hangup_commands() {
        assert_eq!(parse("AT\\B"), vec![SendBreak]);
        assert_eq!(parse("ATO"), vec![Online]);
        assert_eq!(parse("AT H0"), vec![HangUp]);
        assert_eq!(parse("AT"), vec![]);
    }
}
