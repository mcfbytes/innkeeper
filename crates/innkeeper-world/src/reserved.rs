//! Names a player may not take, so nobody passes for the operator or the service in the phone
//! book, a letter or a board post (docs/server/privacy.md section 1).

/// How a reserved word catches a name, once both are folded (`fold`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Catch {
    /// The whole name, or two such words run together ("INN Host").
    Whole,
    /// Also a name that starts or ends with it ("SysopBob", "Bob the Sysop").
    Edge,
    /// Anywhere in the name ("The ImagiNation Network"): only words no ordinary name contains.
    Inside,
}

/// The staff titles of the original service, its names, and the two service boxes the client's own
/// Town Hall addresses by name (`MemberServ`, `AcctUpdate`, docs/server/services.md section 4).
/// Short words stay `Whole` so ordinary names ("Ghost", "Quinn") are not caught.
const RESERVED: &[(&str, Catch)] = &[
    ("sysop", Catch::Inside),
    ("admin", Catch::Edge),
    ("administrator", Catch::Inside),
    ("moderator", Catch::Inside),
    ("staff", Catch::Edge),
    ("guide", Catch::Edge),
    ("imagination", Catch::Inside),
    ("innkeeper", Catch::Inside),
    ("memberserv", Catch::Inside),
    ("memberservices", Catch::Edge),
    ("acctupdate", Catch::Edge),
    ("inn", Catch::Whole),
    ("tsn", Catch::Whole),
    ("host", Catch::Whole),
    ("system", Catch::Whole),
    ("sierra", Catch::Whole),
    ("mod", Catch::Whole),
    ("gm", Catch::Whole),
    ("operator", Catch::Whole),
    ("official", Catch::Whole),
    ("support", Catch::Whole),
    ("accounting", Catch::Whole),
    ("root", Catch::Whole),
    ("network", Catch::Whole),
];

/// True when `name` reads as staff or as the service, whatever its case, spacing, punctuation or
/// look-alike digits ("S.Y.S.0.P", "inn host", "Adm1n").
pub fn is_reserved_name(name: &str) -> bool {
    let folded = fold(name);
    if folded.is_empty() {
        return false;
    }
    let words: Vec<(String, Catch)> = RESERVED
        .iter()
        .map(|(word, catch)| (fold(word), *catch))
        .collect();
    let whole = |text: &str| words.iter().any(|(word, _)| word == text);
    if whole(&folded) {
        return true;
    }
    let edge = words.iter().any(|(word, catch)| match catch {
        Catch::Whole => false,
        Catch::Edge => folded.starts_with(word.as_str()) || folded.ends_with(word.as_str()),
        Catch::Inside => folded.contains(word.as_str()),
    });
    edge || (1..folded.len()).any(|cut| {
        let (head, tail) = folded.split_at(cut);
        whole(head) && whole(tail)
    })
}

/// Lower case, look-alike digits and signs as the letters they imitate, everything else dropped.
fn fold(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c.to_ascii_lowercase() {
            '0' => Some('o'),
            '1' | 'l' | '!' | '|' => Some('i'),
            '3' => Some('e'),
            '4' | '@' => Some('a'),
            '5' | '$' => Some('s'),
            '7' => Some('t'),
            letter if letter.is_ascii_alphanumeric() => Some(letter),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staff_titles_and_service_names_are_reserved_in_any_spelling() {
        for name in [
            "Sysop",
            "SYSOP",
            "s y s o p",
            "S.Y.S.0.P",
            "SysopBob",
            "Bob the Sysop",
            "Admin",
            "Adm1n",
            "Administrator",
            "TSN",
            "T.S.N.",
            "INN",
            "inn host",
            "INN-Host",
            "TSN Staff",
            "Staff",
            "Moderator",
            "Mod",
            "Guide",
            "INN Guide",
            "Host",
            "System",
            "ImagiNation",
            "The ImagiNation Network",
            "Imagination!",
            "MemberServ",
            "Member Services",
            "AcctUpdate",
            "Innkeeper",
            "Sierra",
        ] {
            assert!(is_reserved_name(name), "{name}");
        }
    }

    #[test]
    fn ordinary_names_are_free() {
        for name in [
            "Guybrush",
            "Elaine",
            "Ghost",
            "Quinn",
            "Finn",
            "Hostess",
            "Modesty",
            "Badminton",
            "Ingrid",
            "Tsunami",
            "Sierrafan",
            "Bob",
            "",
            "R2D2",
            "Linnea",
        ] {
            assert!(!is_reserved_name(name), "{name}");
        }
    }
}
