//! Search ranking (spec §9.3): name, category and synonyms, UE-like (`*` = Multiply, `lerp`, `+`). Domain-neutral:
//! a domain's palette entry implements [`Searchable`].

/// What the ranking reads from an item.
pub trait Searchable {
    fn name(&self) -> &str;
    fn category(&self) -> &str;
    fn keywords(&self) -> &[String];
    fn description(&self) -> &str;
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '_').flat_map(char::to_lowercase).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Relevance of `e` for `query` (0 = no match).
pub fn score<E: Searchable>(e: &E, query: &str) -> u32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 1;
    }
    let qs = squash(&q);
    let name = e.name().to_lowercase();
    let sname = squash(e.name());
    let kws: Vec<String> = e.keywords().iter().map(|k| k.to_lowercase()).collect();
    if kws.contains(&q) {
        return 100;
    }
    if sname == qs {
        return 95;
    }
    if sname.starts_with(&qs) {
        return 80;
    }
    if name.split(|c: char| c.is_whitespace() || c == '_').any(|w| w.starts_with(&q)) {
        return 70;
    }
    if kws.iter().any(|k| k.starts_with(&q)) {
        return 60;
    }
    if sname.contains(&qs) {
        return 50;
    }
    if e.category().to_lowercase().contains(&q) {
        return 30;
    }
    if e.description().to_lowercase().contains(&q) {
        return 10;
    }
    if qs.len() >= 2 && is_subsequence(&qs, &sname) {
        return 5;
    }
    0
}

/// Matching entries, best first (stable by category/name on ties).
pub fn search<'a, E: Searchable>(entries: &'a [E], query: &str) -> Vec<&'a E> {
    search_indices(entries, query).into_iter().map(|i| &entries[i]).collect()
}

/// Like [`search`], as indices into `entries`.
pub fn search_indices<E: Searchable>(entries: &[E], query: &str) -> Vec<usize> {
    search_indices_leading(entries, query, |_| false)
}

/// Like [`search_indices`], with the matching entries for which `lead` holds first among those of the same score
/// (the Named Route declaration after a wire from an output, design §6.4: first with an empty query or its own
/// keyword, never above a better match); the rest keeps its order.
pub fn search_indices_leading<E: Searchable>(entries: &[E], query: &str, lead: impl Fn(&E) -> bool) -> Vec<usize> {
    let mut hits: Vec<(u32, bool, usize)> =
        entries.iter().enumerate().map(|(i, e)| (score(e, query), lead(e), i)).filter(|(s, _, _)| *s > 0).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
    hits.into_iter().map(|(_, _, i)| i).collect()
}
