//! Spots one-time codes and sign-in requests in notifications, such as the ones Phone Link
//! forwards from an iPhone or Android phone, so the island can offer to copy the code.
//! Pure text matching: nothing here is logged, stored or sent anywhere.

/// Words that say a number in the message is a code to type in.
const HINTS: [&str; 16] = [
    "code", "otp", "passcode", "password", "pin", "verif", "2fa", "one-time", "one time", "authenticat", "security", "login", "log in", "sign in", "sign-in", "confirm",
];

/// A digit group that might be the code: its digits and where it sits in the text.
struct Cand {
    digits: String,
    start: usize,
    end: usize,
}

/// The one-time code in a notification (an SMS, an email, an app), if it has one: 4 to 8
/// digits near a word like "code" or "verification". "123 456" and "123-456" count as one
/// code; "G-123456" gives "123456". Card endings, amounts, phone numbers, times and dates are
/// skipped. Failing that, a code of letters and digits right after such a word ("code: K7P2QX").
pub fn find_code(texts: &[&str]) -> Option<String> {
    let text = texts.join(" \n ");
    find_digits(&text).or_else(|| find_mixed(&text))
}

/// Lower case with the same byte layout as `text`, so positions match in both.
fn lower_same(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(x), None) if x.len_utf8() == c.len_utf8() => x,
                _ => c,
            }
        })
        .collect()
}

/// A code like "K7P2QX" or "9GH4-T2K": 5 to 8 capitals and digits (both kinds), at most one
/// dash, within a few characters after "code", "OTP" or "PIN".
fn find_mixed(text: &str) -> Option<String> {
    let lower = lower_same(text);
    // Only for sign-in mail, not offers ("use code SAVE20 for 20% off").
    let secure = ["verif", "security", "one-time", "single-use", "login", "log in", "sign in", "sign-in", "confirm", "passcode", "otp", "2fa", "authenticat"];
    let promo = ["promo", "coupon", "discount", "voucher", "% off", "offer", "sale", "deal"];
    if !secure.iter().any(|w| lower.contains(w)) || promo.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let hints: Vec<usize> = ["code", "otp", "pin"].iter().flat_map(|h| lower.match_indices(h).map(move |(i, _)| i + h.len())).collect();
    let ok = |s: usize, e: usize| -> bool {
        let w = &text[s..e];
        let core: String = w.chars().filter(|c| *c != '-').collect();
        (5..=8).contains(&core.chars().count())
            && w.matches('-').count() <= 1
            && !w.starts_with('-')
            && !w.ends_with('-')
            && core.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && core.chars().any(|c| c.is_ascii_digit())
            && core.chars().any(|c| c.is_ascii_uppercase())
            && hints.iter().any(|&h| h <= s && s - h <= 16)
    };
    let mut start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        let word = c.is_ascii_alphanumeric() || c == '-';
        match (word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if ok(s, i) {
                    return Some(text[s..i].to_string());
                }
                start = None;
            }
            _ => {}
        }
    }
    None
}

fn find_digits(text: &str) -> Option<String> {
    let lower = lower_same(text);
    let hints: Vec<usize> = HINTS.iter().flat_map(|h| lower.match_indices(h).map(|(i, _)| i)).collect();
    if hints.is_empty() {
        return None;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    // Runs of digits.
    let mut runs: Vec<(usize, usize)> = Vec::new(); // indices into `chars`, end exclusive
    let mut i = 0;
    while i < chars.len() {
        if chars[i].1.is_ascii_digit() {
            let s = i;
            while i < chars.len() && chars[i].1.is_ascii_digit() {
                i += 1;
            }
            runs.push((s, i));
        } else {
            i += 1;
        }
    }
    // Join runs split by one space or dash into chains ("123 456", "0412 345 678").
    let mut chains: Vec<Vec<(usize, usize)>> = Vec::new();
    for r in runs {
        if let Some(last) = chains.last_mut() {
            let prev = *last.last().unwrap();
            if r.0 == prev.1 + 1 && matches!(chars[prev.1].1, ' ' | '-') {
                last.push(r);
                continue;
            }
        }
        chains.push(vec![r]);
    }
    let at = |k: usize| chars.get(k).map(|c| c.1);
    let before = |k: usize| if k == 0 { None } else { at(k - 1) };
    let mut cands: Vec<Cand> = Vec::new();
    for chain in chains {
        let lens: Vec<usize> = chain.iter().map(|r| r.1 - r.0).collect();
        let pieces: Vec<(usize, usize)> = match lens.as_slice() {
            // "123 456" / "123-456" / "1234 5678": one code written in two halves.
            [a, b] if a == b && (*a == 3 || *a == 4) => vec![(chain[0].0, chain[1].1)],
            [_] => chain.clone(),
            // Three or more groups: a phone or card number, not a code.
            _ if chain.len() >= 3 => continue,
            _ => chain.clone(),
        };
        for (s, e) in pieces {
            let digits: String = chars[s..e].iter().map(|c| c.1).filter(|c| c.is_ascii_digit()).collect();
            if !(4..=8).contains(&digits.len()) {
                continue;
            }
            // Glued to letters ("A1234"), a decimal or thousands part ("1,234.50"), a time
            // ("10:30"), a date ("12/2026"), money, a percentage, or a phone number.
            let b = before(s);
            let a = at(e);
            if b.is_some_and(|c| c.is_alphanumeric() || matches!(c, '$' | '€' | '£' | '¥' | '₹' | '+' | '.' | ',' | ':' | '/' | '#' | '*')) {
                continue;
            }
            if a.is_some_and(|c| c.is_alphanumeric() || c == '%')
                || (matches!(a, Some('.' | ',' | ':' | '/')) && at(e + 1).is_some_and(|c| c.is_ascii_digit()))
            {
                continue;
            }
            // "G-123456" is fine; "ABC-123456" is a reference number.
            let alnum = |k: usize| at(k).is_some_and(|c| c.is_alphanumeric());
            if b == Some('-') && s >= 3 && alnum(s - 2) && alnum(s - 3) {
                continue;
            }
            let start = chars[s].0;
            let end = chars.get(e).map_or(text.len(), |c| c.0);
            let lead = lower[..start].trim_end();
            if ["ending in", "ending", "ends in", "card", "acct", "account", "a/c", "ref", "order", "no.", "number", "$", "aud", "usd"].iter().any(|w| lead.ends_with(w)) {
                continue;
            }
            // A 4-digit year next to a month or "in 2026" is a date.
            if digits.len() == 4 && (digits.starts_with("19") || digits.starts_with("20")) {
                let near: String = lower[..start].chars().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect();
                if ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec", " in ", "©"].iter().any(|m| near.contains(m)) {
                    continue;
                }
            }
            cands.push(Cand { digits, start, end });
        }
    }
    // The candidate closest to a hint word; six digits wins a tie.
    let dist = |c: &Cand| hints.iter().map(|&h| if h >= c.end { h - c.end } else { c.start.saturating_sub(h) }).min().unwrap_or(usize::MAX);
    let best = cands.into_iter().filter(|c| dist(c) <= 80).min_by_key(|c| (dist(c) / 12, (c.digits.len() != 6) as u8, dist(c)))?;
    Some(best.digits)
}

/// An Okta Verify push ("Did you just try to sign in?"): true when the notification asks you
/// to approve a sign-in. Approving stays on the phone, where Okta checks it really is you.
pub fn is_okta_push(texts: &[&str]) -> bool {
    let t = texts.join(" ").to_lowercase();
    t.contains("okta")
        && ["sign in", "sign-in", "signing in", "signed in", "trying to", "is it you", "approve", "verify", "push", "request"].iter().any(|w| t.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(s: &str) -> Option<String> {
        find_code(&["Messages", s])
    }

    #[test]
    fn finds_codes() {
        assert_eq!(code("Your verification code is 482913. It expires in 10 minutes."), Some("482913".into()));
        assert_eq!(code("G-482913 is your Google verification code."), Some("482913".into()));
        assert_eq!(code("Use 4829 to log in to Uber. Never share it."), Some("4829".into()));
        assert_eq!(code("Your OTP: 123 456"), Some("123456".into()));
        assert_eq!(code("Your OTP: 123-456"), Some("123456".into()));
        assert_eq!(code("CommBank: NetCode 73810244 for your transfer of $1,250.00"), Some("73810244".into()));
        assert_eq!(code("WhatsApp code 381-204. Don't share this code with others"), Some("381204".into()));
        assert_eq!(find_code(&["Okta", "Your Okta verification code is 591027"]), Some("591027".into()));
        assert_eq!(code("Microsoft account security code: 7302"), Some("7302".into()));
    }

    #[test]
    fn email_codes() {
        // Outlook, Gmail through the browser, Apple Mail, and a phone's mail app via Phone Link.
        let outlook = ["Outlook", "Microsoft account team", "Your single-use code · Hi, we received your request for a single-use code to use with your Microsoft account. Your single-use code is: 284019"];
        assert_eq!(find_code(&outlook), Some("284019".into()));
        assert_eq!(find_code(&["Google Chrome", "Instagram", "482913 is your Instagram code · Hi, someone tried to log in to your account"]), Some("482913".into()));
        assert_eq!(find_code(&["Mail", "Apple", "Your Apple Account Code · Your verification code is: 507 239"]), Some("507239".into()));
        assert_eq!(find_code(&["Phone Link", "Gmail", "Verify your email · Thanks for signing up! Please enter the following code in the app to finish creating your account. 739201"]), Some("739201".into()));
        assert_eq!(find_code(&["Outlook", "Bank", "Sign-in attempt on 18 Nov 2026 at 7:42 PM. Your code is 663910"]), Some("663910".into()));
        assert_eq!(find_code(&["Outlook", "Amazon", "Your Amazon OTP is 448201. Order #112-3345678-1234567"]), Some("448201".into()));
        assert_eq!(find_code(&["Outlook", "UNSW", "Your code is 551203. UNSW Sydney NSW 2052"]), Some("551203".into()));
        // Codes with letters.
        assert_eq!(find_code(&["Outlook", "Epic Games", "Your security code: QX7R2M"]), Some("QX7R2M".into()));
        assert_eq!(find_code(&["Slack", "Your confirmation code is 9GH4-T2K"]), Some("9GH4-T2K".into()));
        assert_eq!(find_code(&["Outlook", "Newsletter", "Use code SAVE20 for 20% off"]), None);
        assert_eq!(find_code(&["Outlook", "Weekly digest", "NSW COVID update and more"]), None);
    }

    #[test]
    fn skips_other_numbers() {
        assert_eq!(code("Lunch at 1230? call me on 0412 345 678"), None);
        assert_eq!(code("Your card ending in 4821 was charged $52.10. Not you? call 1300 123 456"), None);
        assert_eq!(code("Payment of $1,250.00 received. Ref 99812345"), None);
        assert_eq!(code("Your code is 2 days late"), None);
        assert_eq!(code("Meeting moved to 10:30 tomorrow"), None);
        // An amount beside the code doesn't win.
        assert_eq!(code("Code 662091 to approve your payment of $1999"), Some("662091".into()));
        assert_eq!(code("Code for your 2026 login: 448120"), Some("448120".into()));
        assert_eq!(code("Order 12345678 confirmed"), None);
    }

    #[test]
    fn okta_push() {
        assert!(is_okta_push(&["Okta Verify", "Did you just try to sign in?", "Acme · Sydney, AU"]));
        assert!(!is_okta_push(&["Messages", "Did you just try to sign in?"]));
        assert!(!is_okta_push(&["Okta", "Your password expires in 5 days"]));
    }
}
