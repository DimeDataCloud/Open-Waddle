//! Making sense of what was heard. Speech-to-text writes app names the way
//! they sound ("Clawed" for Claude, "note pad"), wraps sentences in quotes and
//! keeps the "uh"s. These helpers tidy a spoken message and match a name
//! against the installed apps by spelling and by sound.

/// Words that carry nothing when spoken.
const FILLERS: &[&str] = &["uh", "uhh", "uhm", "um", "umm", "erm", "er", "hmm", "mm"];

/// A message as it should reach Waddle: fillers dropped and, when it was spoken,
/// the quotation marks dictation adds removed. Nothing else changes.
pub fn tidy(text: &str, spoken: bool) -> String {
    let mut words: Vec<String> = vec![];
    for raw in text.split_whitespace() {
        let token = if spoken { raw.replace(['"', '“', '”'], "") } else { raw.to_string() };
        let bare: String = token.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        if FILLERS.contains(&bare.as_str()) {
            // "Uh, can you" loses the comma with the filler; "open it. Umm" keeps the full stop.
            if let Some(prev) = words.last_mut() {
                let end: String = token.chars().rev().take_while(|c| matches!(c, '.' | '?' | '!')).collect();
                if !end.is_empty() && !prev.ends_with(['.', '?', '!']) {
                    prev.push_str(&end);
                }
            }
            continue;
        }
        if !token.is_empty() {
            words.push(token);
        }
    }
    let mut out = words.join(" ");
    // "Uh, can you…" → "Can you…"
    if let Some(first) = out.chars().next() {
        if first.is_lowercase() && text.trim_start().chars().next().is_some_and(|c| c.is_uppercase() || c == '"' || c == '“') {
            out = first.to_uppercase().collect::<String>() + &out[first.len_utf8()..];
        }
    }
    let out = out.trim_start_matches([',', ' ']).to_string();
    if out.is_empty() {
        text.trim().to_string()
    } else {
        out
    }
}

/// American Soundex: names that sound alike share a code ("Clawed" and "Claude" are C430).
pub fn soundex(word: &str) -> String {
    let letters: Vec<char> = word.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase()).collect();
    let Some(&first) = letters.first() else { return String::new() };
    let code = |c: char| match c {
        'B' | 'F' | 'P' | 'V' => '1',
        'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => '2',
        'D' | 'T' => '3',
        'L' => '4',
        'M' | 'N' => '5',
        'R' => '6',
        'H' | 'W' => '-', // ignored, and they don't separate equal codes
        _ => '0',         // vowels separate equal codes
    };
    let mut out = String::from(first);
    let mut last = code(first);
    for &c in &letters[1..] {
        let k = code(c);
        match k {
            '-' => continue,
            '0' => last = '0',
            _ if k != last => {
                out.push(k);
                last = k;
            }
            _ => {}
        }
        if out.len() == 4 {
            break;
        }
    }
    while out.len() < 4 {
        out.push('0');
    }
    out
}

/// Edit distance (insertions, deletions, substitutions).
fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// An app name in a comparable form: lower case, no punctuation, without "the" and "app".
fn normalise(name: &str) -> String {
    let cleaned: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.first() == Some(&"the") {
        words.remove(0);
    }
    while words.len() > 1 && matches!(words.last(), Some(&("app" | "application" | "program" | "desktop"))) {
        words.pop();
    }
    words.join(" ")
}

/// How well `query` names `candidate`, from 0 (not at all) to 1 (exactly).
fn score(query: &str, candidate: &str) -> f64 {
    let (q, c) = (normalise(query), normalise(candidate));
    if q.is_empty() || c.is_empty() {
        return 0.0;
    }
    let (qs, cs) = (q.replace(' ', ""), c.replace(' ', ""));
    if q == c || qs == cs {
        return 1.0;
    }
    let mut best: f64 = 0.0;
    // "antigravity ide" for Antigravity; "visual studio" for Visual Studio Code.
    if q.starts_with(&format!("{c} ")) {
        best = best.max(0.9);
    }
    if c.starts_with(&format!("{q} ")) {
        best = best.max(0.85);
    }
    // The start of the name: "calc" for Calculator.
    if qs.chars().count() >= 4 && cs.starts_with(&qs) {
        best = best.max(0.9);
    }
    let longest = qs.chars().count().max(cs.chars().count()) as f64;
    best = best.max(1.0 - levenshtein(&qs, &cs) as f64 / longest);
    // Sounds the same and is about as long: "Clawed" and "Claude".
    let ratio = qs.len().min(cs.len()) as f64 / qs.len().max(cs.len()) as f64;
    if qs.len() >= 4 && ratio >= 0.7 && soundex(&qs) == soundex(&cs) {
        best = best.max(0.85);
    }
    best
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMatch {
    /// One installed app is clearly what was meant.
    Clear(String),
    /// Nothing clear; these are the closest (best first, possibly none).
    Unsure(Vec<String>),
}

/// The installed app a heard or misspelled name most likely means.
pub fn match_app(query: &str, names: &[String]) -> AppMatch {
    let mut scored: Vec<(f64, &String)> = names.iter().map(|n| (score(query, n), n)).filter(|(s, _)| *s > 0.0).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
    scored.dedup_by(|a, b| a.1.eq_ignore_ascii_case(b.1));
    match scored.as_slice() {
        [(best, name), rest @ ..] if *best >= 0.8 && rest.first().is_none_or(|(next, _)| *next < best - 0.04) => AppMatch::Clear((*name).clone()),
        _ => AppMatch::Unsure(scored.iter().filter(|(s, _)| *s >= 0.45).take(3).map(|(_, n)| (*n).clone()).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<String> {
        ["Clock", "Calculator", "Claude", "Google Chrome", "Camera", "Calendar", "Antigravity", "Visual Studio Code", "Notepad", "Spotify", "Firefox", "GitHub Desktop"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn misheard_app_names_find_the_app() {
        let apps = apps();
        assert_eq!(match_app("Clawed", &apps), AppMatch::Clear("Claude".into()));
        assert_eq!(match_app("the Clawed desktop app", &apps), AppMatch::Clear("Claude".into()));
        assert_eq!(match_app("antigravity IDE", &apps), AppMatch::Clear("Antigravity".into()));
        assert_eq!(match_app("note pad", &apps), AppMatch::Clear("Notepad".into()));
        assert_eq!(match_app("Fire fox", &apps), AppMatch::Clear("Firefox".into()));
        assert_eq!(match_app("Spotify app", &apps), AppMatch::Clear("Spotify".into()));
        assert_eq!(match_app("visual studio", &apps), AppMatch::Clear("Visual Studio Code".into()));
        assert_eq!(match_app("github", &apps), AppMatch::Clear("GitHub Desktop".into()));
    }

    #[test]
    fn unclear_names_suggest_instead_of_guessing() {
        let apps = apps();
        assert_eq!(match_app("banana", &apps), AppMatch::Unsure(vec![]));
        assert_eq!(match_app("Calc", &apps), AppMatch::Clear("Calculator".into()));
        // Two apps that fit equally well: ask which, instead of picking one.
        let twins = vec!["Clock Pro".to_string(), "Clock Plus".to_string()];
        assert_eq!(match_app("Clock", &twins), AppMatch::Unsure(vec!["Clock Pro".into(), "Clock Plus".into()]));
        // A plain misspelling of one of them is clear.
        assert_eq!(match_app("Clok", &["Clock".to_string(), "Klok".to_string()]), AppMatch::Clear("Clock".into()));
    }

    #[test]
    fn soundex_matches_names_that_sound_alike() {
        assert_eq!(soundex("Clawed"), "C430");
        assert_eq!(soundex("Claude"), "C430");
        assert_eq!(soundex("Robert"), "R163");
        assert_eq!(soundex("Rupert"), "R163");
        assert_eq!(soundex("Ashcraft"), "A261");
        assert_eq!(soundex("Tymczak"), "T522");
        assert_eq!(soundex(""), "");
    }

    #[test]
    fn spoken_messages_lose_fillers_and_dictation_quotes() {
        assert_eq!(tidy("Uh, can you open up YouTube?", true), "Can you open up YouTube?");
        assert_eq!(tidy("\"Fly around the screen.\"", true), "Fly around the screen.");
        assert_eq!(
            tidy("Can you open up Youtube music? Uh, and just pick a random playlist", true),
            "Can you open up Youtube music? and just pick a random playlist"
        );
        assert_eq!(tidy("um play the video by adapt live", true), "play the video by adapt live");
        assert_eq!(tidy("Never mind. Umm just go ahead and open up, uh, a Chrome browser?", true), "Never mind. just go ahead and open up, a Chrome browser?");
        // Typed messages keep their quotes (an exact search, a subject line).
        assert_eq!(tidy("search for \"rust async\"", false), "search for \"rust async\"");
        assert_eq!(tidy("um", true), "um", "never empties a message");
        assert_eq!(tidy("Summarize the umbrella report", true), "Summarize the umbrella report");
    }
}
