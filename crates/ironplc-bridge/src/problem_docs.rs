//! Embedded copy of ironplc's problem documentation.
//!
//! The build.rs generates a `lookup_problem_doc(code) -> Option<(rst, title)>`
//! function with one match arm per `vendor/ironplc/docs/reference/compiler/
//! problems/P####.rst`. The RST bodies are `include_str!`'d so they live
//! in the binary — no filesystem access at runtime, no version drift
//! against the vendor checkout.
//!
//! Why we don't just reuse `ironplc-mcp`'s copy: that crate is part of
//! the vendor tree and we'd have to depend on it (currently we don't),
//! pulling in `rmcp` and the rest of the MCP server surface. The
//! ~50-line build script + this stub is cheaper and stays in line
//! with our "don't import what you don't use" instinct.

include!(concat!(env!("OUT_DIR"), "/problem_docs.rs"));

/// Return only the body (no title) — convenient for callers that just
/// want the prose to dump into a diagnostic payload. Strips the RST
/// title block (`===\nP####\n===` lines) so the body reads as the
/// "explanation" without the redundant header.
pub fn lookup_problem_explanation(code: &str) -> Option<String> {
    let (rst, _title) = lookup_problem_doc(code)?;
    Some(strip_rst_title_block(rst))
}

/// Remove the leading reStructuredText title block from a doc page.
/// RST titles look like:
///
/// ```text
/// =====
/// P4007
/// =====
///
/// (body…)
/// ```
///
/// The `===` overlines/underlines + the title line are redundant when
/// the diagnostic already carries the code as a structured field.
fn strip_rst_title_block(rst: &str) -> String {
    let mut lines = rst.lines().peekable();
    // Pattern A: overline + title + underline (three lines starting
    // with at least one `=`).
    if let (Some(a), Some(_b), Some(c)) = (
        lines.peek().copied(),
        {
            let mut clone = rst.lines();
            clone.next();
            clone.next()
        },
        {
            let mut clone = rst.lines();
            clone.next();
            clone.nth(1)
        },
    ) {
        if a.trim_start().starts_with('=') && c.trim_start().starts_with('=') {
            // Skip three header lines + any blank lines immediately after.
            lines.next();
            lines.next();
            lines.next();
            while matches!(lines.peek().map(|s| s.trim().is_empty()), Some(true)) {
                lines.next();
            }
            return lines.collect::<Vec<_>>().join("\n");
        }
    }
    // No title block detected; return as-is.
    rst.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_p4007() {
        let (rst, title) = lookup_problem_doc("P4007").expect("P4007 should exist");
        assert!(!rst.is_empty());
        assert!(!title.is_empty());
        // The title from the CSV is the human description.
        assert!(
            title.to_lowercase().contains("variable") || title.to_lowercase().contains("not"),
            "got title: {title}"
        );
    }

    #[test]
    fn unknown_code_returns_none() {
        // Pick something well outside ironplc's allocated range (P0–P9999).
        assert!(lookup_problem_doc("XX-INVALID").is_none());
        assert!(lookup_problem_doc("").is_none());
        assert!(lookup_problem_doc("P10000").is_none());
    }

    #[test]
    fn explanation_strips_rst_title_block() {
        let body = lookup_problem_explanation("P4007").expect("P4007 should exist");
        // Body should NOT start with the `===` overline.
        assert!(
            !body.trim_start().starts_with('='),
            "body should have its title stripped; got start: {:?}",
            &body[..40.min(body.len())]
        );
        // And it should still contain real content — at minimum the
        // word "variable" or similar.
        assert!(body.to_lowercase().contains("variable"));
    }
}
