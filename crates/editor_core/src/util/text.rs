//! Text helpers.

/// `n` and the noun in the right number: `1 node`, `2 nodes`, `0 nodes`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::plural;

    #[test]
    fn plural_forms() {
        assert_eq!(plural(1, "node", "nodes"), "1 node");
        assert_eq!(plural(2, "node", "nodes"), "2 nodes");
        assert_eq!(plural(0, "node", "nodes"), "0 nodes");
    }
}
