//! Names safe for branches, folders and file names.

/// Lowercase ASCII words joined by '-', safe for branch and directory names.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "agent".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Fix the Login bug!"), "fix-the-login-bug");
        assert_eq!(slug("  ---  "), "agent");
        assert_eq!(slug("ünï"), "n");
        assert_eq!(slug("ü"), "agent");
        assert!(slug(&"ab ".repeat(50)).len() <= 40);
        assert!(!slug(&"ab ".repeat(50)).ends_with('-'));
    }
}
