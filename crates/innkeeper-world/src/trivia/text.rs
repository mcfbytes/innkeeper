//! Question and answer text as the client draws it: 24-byte rows, ASCII only.

/// Type 4 and type 5 slice length (`proc_339`, `proc_340`).
pub(crate) const LINE: usize = 24;
/// Type 4 carries this many question rows.
const QUESTION_ROWS: usize = 4;

/// One question that already fits the NTN rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub lines: [[u8; LINE]; QUESTION_ROWS],
    /// Index 0 is the correct answer; the show shuffles a copy when it asks.
    pub answers: Vec<[u8; LINE]>,
}

/// A question as the JSON banks store it, before wrapping and the ASCII filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawQuestion {
    pub question: String,
    pub correct: String,
    pub incorrect: Vec<String>,
    pub boolean: bool,
}

/// Wrap, transliterate, and drop anything that still will not fit the rows.
pub fn fit(raw: &RawQuestion) -> Option<Item> {
    let question = ascii(&raw.question)?;
    let lines = wrap(&question)?;
    let answers = if raw.boolean {
        boolean_answers(&raw.correct)?
    } else {
        choice_answers(&raw.correct, &raw.incorrect)?
    };
    Some(Item { lines, answers })
}

fn boolean_answers(correct: &str) -> Option<Vec<[u8; LINE]>> {
    let right = ascii(correct)?;
    let true_line = pad("True")?;
    let false_line = pad("False")?;
    match right.as_str() {
        "True" | "true" => Some(vec![true_line, false_line]),
        "False" | "false" => Some(vec![false_line, true_line]),
        _ => None,
    }
}

fn choice_answers(correct: &str, incorrect: &[String]) -> Option<Vec<[u8; LINE]>> {
    let mut answers = Vec::new();
    answers.push(pad(&ascii(correct)?)?);
    for wrong in incorrect {
        answers.push(pad(&ascii(wrong)?)?);
        if answers.len() == 5 {
            break;
        }
    }
    (2..=5).contains(&answers.len()).then_some(answers)
}

fn pad(text: &str) -> Option<[u8; LINE]> {
    let bytes = text.as_bytes();
    if bytes.len() > LINE {
        return None;
    }
    let graphic = bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ');
    if !graphic {
        return None;
    }
    let mut out = [b' '; LINE];
    out.get_mut(..bytes.len())?.copy_from_slice(bytes);
    Some(out)
}

/// Four space-padded rows, or none when the text needs a fifth.
pub fn wrap(text: &str) -> Option<[[u8; LINE]; QUESTION_ROWS]> {
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if word.len() > LINE {
            if !current.is_empty() {
                rows.push(std::mem::take(&mut current));
            }
            for chunk in word.as_bytes().chunks(LINE) {
                rows.push(String::from_utf8_lossy(chunk).into_owned());
            }
            continue;
        }
        let gap = usize::from(!current.is_empty());
        if current.len() + gap + word.len() > LINE {
            rows.push(std::mem::take(&mut current));
            current.push_str(word);
        } else {
            if gap == 1 {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() || rows.len() > QUESTION_ROWS {
        return None;
    }
    let mut out = [[b' '; LINE]; QUESTION_ROWS];
    for (index, row) in rows.iter().enumerate() {
        let bytes = row.as_bytes();
        out.get_mut(index)?
            .get_mut(..bytes.len())?
            .copy_from_slice(bytes);
    }
    Some(out)
}

fn ascii(text: &str) -> Option<String> {
    let decoded = entities(text);
    let mut out = String::new();
    for ch in decoded.chars() {
        fold(&mut out, ch)?;
    }
    let trimmed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!trimmed.is_empty()).then_some(trimmed)
}

fn entities(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(rest.get(..start).unwrap_or(""));
        let tail = rest.get(start..).unwrap_or("");
        match entity_at(tail) {
            Some((value, skip)) => {
                out.push_str(value);
                rest = tail.get(skip..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity_at(text: &str) -> Option<(&'static str, usize)> {
    const PAIRS: &[(&str, &str)] = &[
        ("&ldquo;", "\""),
        ("&rdquo;", "\""),
        ("&lsquo;", "'"),
        ("&rsquo;", "'"),
        ("&nbsp;", " "),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&#039;", "'"),
        ("&amp;", "&"),
        ("&#39;", "'"),
        ("&lt;", "<"),
        ("&gt;", ">"),
    ];
    let end = text.find(';')? + 1;
    let entity = text.get(..end)?;
    PAIRS
        .iter()
        .find(|(token, _)| *token == entity)
        .map(|(_, value)| (*value, end))
}

fn fold(out: &mut String, ch: char) -> Option<()> {
    if ch.is_ascii() {
        match ch {
            '\0' => return None,
            '\n' | '\r' | '\t' => out.push(' '),
            other => out.push(other),
        }
        return Some(());
    }
    out.push_str(match ch {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "A",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ė' | 'Ę' | 'Ě' => "E",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'Į' | 'İ' => "I",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => "i",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' | 'Ů' | 'Ű' => "U",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
        'Ý' | 'Ÿ' => "Y",
        'ý' | 'ÿ' => "y",
        'Ç' | 'Ć' | 'Č' => "C",
        'ç' | 'ć' | 'č' => "c",
        'Ñ' | 'Ń' | 'Ň' => "N",
        'ñ' | 'ń' | 'ň' => "n",
        'Ś' | 'Š' => "S",
        'ś' | 'š' => "s",
        'Ž' | 'Ź' | 'Ż' => "Z",
        'ž' | 'ź' | 'ż' => "z",
        'Ł' => "L",
        'ł' => "l",
        'ß' => "ss",
        'Æ' => "AE",
        'æ' => "ae",
        'Œ' => "OE",
        'œ' => "oe",
        'Ð' => "D",
        'ð' => "d",
        'Þ' => "Th",
        'þ' => "th",
        '’' | '‘' | '‚' => "'",
        '“' | '”' | '„' => "\"",
        '–' | '—' => "-",
        '…' => "...",
        '×' => "x",
        '÷' => "/",
        '\u{00a0}' => " ",
        _ => return None,
    });
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(question: &str, correct: &str, wrong: &[&str]) -> RawQuestion {
        RawQuestion {
            question: question.into(),
            correct: correct.into(),
            incorrect: wrong.iter().map(|s| (*s).to_string()).collect(),
            boolean: false,
        }
    }

    fn row(text: &str) -> [u8; LINE] {
        pad(text).unwrap()
    }

    #[test]
    fn a_short_question_fills_four_space_padded_rows() {
        let item = fit(&raw(
            "Who founded Sierra?",
            "Ken Williams",
            &["Al Lowe", "Ron Gilbert"],
        ))
        .unwrap();
        assert_eq!(item.lines[0], row("Who founded Sierra?"));
        assert_eq!(item.lines[1], [b' '; LINE]);
        assert_eq!(item.answers[0], row("Ken Williams"));
    }

    #[test]
    fn wrapping_breaks_on_spaces_and_rejects_a_fifth_line() {
        let four = "one two three four five six seven eight nine ten eleven twelve thirteen";
        let lines = wrap(four).unwrap();
        assert_eq!(lines[0], row("one two three four five"));
        assert_eq!(lines[1], row("six seven eight nine ten"));
        assert!(wrap(&format!("{four} fourteen fifteen sixteen seventeen")).is_none());
    }

    #[test]
    fn accents_become_ascii_and_leftover_unicode_is_dropped() {
        assert!(fit(&raw(
            "Who designed Myst?",
            "Rand Miller",
            &["Robyn Miller", "Cyan"],
        ))
        .is_some());
        assert_eq!(ascii("Café résumé").unwrap(), "Cafe resume");
        assert!(ascii("who ★ this").is_none());
        assert_eq!(ascii("It&#039;s &quot;fun&quot;").unwrap(), "It's \"fun\"");
    }

    #[test]
    fn answers_longer_than_24_do_not_fit() {
        assert!(fit(&raw(
            "Short?",
            "This answer is much too long for NTN",
            &["No", "Yes"]
        ))
        .is_none());
        let boolean = RawQuestion {
            question: "Is this true?".into(),
            correct: "True".into(),
            incorrect: vec!["False".into()],
            boolean: true,
        };
        let item = fit(&boolean).unwrap();
        assert_eq!(item.answers.len(), 2);
        assert_eq!(item.answers[0], row("True"));
    }
}
