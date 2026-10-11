//! Sub-items. Each day's items are one list in outline order, each with a
//! `depth`: an item's sub-items are the items right after it that are
//! deeper, up to the next one that isn't. An item and its sub-items are its
//! block, which moves, completes and deletes as one. Only top-level items
//! (depth 0) decide which days an item shows on; sub-items go with them.

use crate::store::Item;

/// Where the block of the item at `i` ends: the first index after it that
/// isn't one of its sub-items.
pub fn block_end(items: &[Item], i: usize) -> usize {
    let depth = items[i].depth;
    items[i + 1..].iter().position(|item| item.depth <= depth).map_or(items.len(), |n| i + 1 + n)
}

/// The item that `i` is a sub-item of, if any.
pub fn parent(items: &[Item], i: usize) -> Option<usize> {
    let depth = items[i].depth;
    (depth > 0).then(|| items[..i].iter().rposition(|item| item.depth < depth)).flatten()
}

/// Every item that `i` is under, nearest first.
pub fn ancestors(items: &[Item], i: usize) -> Vec<usize> {
    let mut found = Vec::new();
    let mut at = i;
    while let Some(up) = parent(items, at) {
        found.push(up);
        at = up;
    }
    found
}

/// Whether `i` is hidden by a folded item above it.
pub fn hidden(items: &[Item], i: usize) -> bool {
    ancestors(items, i).iter().any(|&up| items[up].folded)
}

/// The indices in the block starting at `i` that show: `i` itself, then its
/// sub-items unless something folds them away.
pub fn visible_block(items: &[Item], i: usize) -> Vec<usize> {
    let end = block_end(items, i);
    let mut shown = vec![i];
    let mut at = i + 1;
    while at < end {
        shown.push(at);
        at = if items[at].folded { block_end(items, at) } else { at + 1 };
    }
    if items[i].folded { vec![i] } else { shown }
}

/// The direct sub-items of `i`.
pub fn children(items: &[Item], i: usize) -> Vec<usize> {
    let end = block_end(items, i);
    let mut found = Vec::new();
    let mut at = i + 1;
    while at < end {
        found.push(at);
        at = block_end(items, at);
    }
    found
}

/// How many of `i`'s direct sub-items are done, and how many it has.
pub fn progress(items: &[Item], i: usize) -> (usize, usize) {
    let children = children(items, i);
    (children.iter().filter(|&&c| items[c].done).count(), children.len())
}

/// The item before `i` at its level, under the same parent, if any.
pub fn prev_sibling(items: &[Item], i: usize) -> Option<usize> {
    let depth = items[i].depth;
    let j = items[..i].iter().rposition(|item| item.depth <= depth)?;
    (items[j].depth == depth).then_some(j)
}

/// The item after `i`'s block at its level, under the same parent, if any.
pub fn next_sibling(items: &[Item], i: usize) -> Option<usize> {
    let end = block_end(items, i);
    (end < items.len() && items[end].depth == items[i].depth).then_some(end)
}

/// The top-level items, in order.
pub fn roots(items: &[Item]) -> Vec<usize> {
    items.iter().enumerate().filter(|(_, item)| item.depth == 0).map(|(i, _)| i).collect()
}

/// Shifts the depths of `items` so the shallowest is `depth`, keeping how
/// they nest.
pub fn rebase(items: &mut [Item], depth: usize) {
    let shallowest = items.iter().map(|item| item.depth).min().unwrap_or(0);
    for item in items.iter_mut() {
        item.depth = item.depth - shallowest + depth;
    }
}

/// Puts a day's items back in order after a change: an item with an open
/// sub-item is open too, and completed top-level items come after the open
/// ones, each group keeping its order. Returns where each item moved to,
/// by its old index.
pub fn normalize(items: &mut Vec<Item>) -> Vec<usize> {
    for i in 0..items.len() {
        if !items[i].done {
            for up in ancestors(items, i) {
                items[up].done = false;
            }
        }
    }
    let mut blocks: Vec<(usize, usize)> = roots(items).into_iter().map(|start| (start, block_end(items, start))).collect();
    // A list that starts with sub-items (from a hand-edited file) keeps them
    // as a block of their own at the front.
    if let Some(&(first, _)) = blocks.first()
        && first > 0
    {
        blocks.insert(0, (0, first));
    }
    if items.is_empty() {
        return Vec::new();
    }
    blocks.sort_by_key(|&(start, _)| items[start].done && items[start].depth == 0);
    let order: Vec<usize> = blocks.into_iter().flat_map(|(start, end)| start..end).collect();
    let mut moved = vec![0; items.len()];
    for (new, &old) in order.iter().enumerate() {
        moved[old] = new;
    }
    let mut taken: Vec<Option<Item>> = std::mem::take(items).into_iter().map(Some).collect();
    *items = order.into_iter().map(|old| taken[old].take().expect("each item once")).collect();
    moved
}

/// The indices among `indices` that aren't inside another one's block, so
/// acting on each acts on every selected item once.
pub fn outermost(items: &[Item], indices: &[usize]) -> Vec<usize> {
    let mut sorted: Vec<usize> = indices.iter().copied().filter(|&i| i < items.len()).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut kept: Vec<usize> = Vec::new();
    for i in sorted {
        if !kept.iter().any(|&k| i < block_end(items, k)) {
            kept.push(i);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Items from `text`, one per line, two spaces of indent per level and
    /// `x ` before a completed one.
    fn outline(text: &str) -> Vec<Item> {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let depth = (line.len() - line.trim_start().len()) / 2;
                let rest = line.trim_start();
                let (done, text) = match rest.strip_prefix("x ") {
                    Some(text) => (true, text),
                    None => (false, rest),
                };
                Item { text: text.into(), done, depth, ..Item::default() }
            })
            .collect()
    }

    fn texts(items: &[Item]) -> Vec<String> {
        items.iter().map(|item| format!("{}{}{}", "  ".repeat(item.depth), if item.done { "x " } else { "" }, item.text)).collect()
    }

    const TRIP: &str = "
trip
  flights
  hotel
    compare
    book
milk
";

    #[test]
    fn blocks_parents_and_roots() {
        let items = outline(TRIP);
        assert_eq!(block_end(&items, 0), 5);
        assert_eq!(block_end(&items, 2), 5);
        assert_eq!(block_end(&items, 1), 2);
        assert_eq!(block_end(&items, 5), 6);
        assert_eq!(parent(&items, 3), Some(2));
        assert_eq!(parent(&items, 2), Some(0));
        assert_eq!(parent(&items, 0), None);
        assert_eq!(ancestors(&items, 4), [2, 0]);
        assert_eq!(roots(&items), [0, 5]);
        assert_eq!(children(&items, 0), [1, 2]);
        assert_eq!(children(&items, 2), [3, 4]);
        assert!(children(&items, 1).is_empty());
    }

    #[test]
    fn siblings_share_a_parent_and_a_level() {
        let items = outline(TRIP);
        assert_eq!(next_sibling(&items, 1), Some(2));
        assert_eq!(prev_sibling(&items, 2), Some(1));
        assert_eq!(prev_sibling(&items, 1), None, "the first sub-item");
        assert_eq!(next_sibling(&items, 4), None, "the last sub-item");
        assert_eq!(next_sibling(&items, 0), Some(5));
        assert_eq!(prev_sibling(&items, 5), Some(0));
        assert_eq!(prev_sibling(&items, 3), None);
    }

    #[test]
    fn folding_hides_what_is_under_an_item() {
        let mut items = outline(TRIP);
        items[2].folded = true;
        assert_eq!(visible_block(&items, 0), [0, 1, 2]);
        assert!(hidden(&items, 3) && hidden(&items, 4));
        assert!(!hidden(&items, 2));
        items[0].folded = true;
        assert_eq!(visible_block(&items, 0), [0]);
    }

    #[test]
    fn progress_counts_direct_sub_items() {
        let items = outline("trip\n  x flights\n  hotel\n    x compare\n");
        assert_eq!(progress(&items, 0), (1, 2));
        assert_eq!(progress(&items, 2), (1, 1));
        assert_eq!(progress(&items, 1), (0, 0));
    }

    #[test]
    fn an_open_sub_item_reopens_what_its_under() {
        let mut items = outline("x trip\n  x flights\n  hotel\nmilk\n");
        normalize(&mut items);
        assert_eq!(texts(&items), ["trip", "  x flights", "  hotel", "milk"]);
    }

    #[test]
    fn completed_top_level_blocks_come_last_in_order() {
        let mut items = outline("x a\n  x a1\nb\nx c\nd\n  d1\n");
        let moved = normalize(&mut items);
        assert_eq!(texts(&items), ["b", "d", "  d1", "x a", "  x a1", "x c"]);
        assert_eq!(moved, [3, 4, 0, 5, 1, 2]);
    }

    #[test]
    fn rebase_keeps_nesting() {
        let mut items = outline("    a\n      b\n    c\n");
        rebase(&mut items, 1);
        assert_eq!(texts(&items), ["  a", "    b", "  c"]);
    }

    #[test]
    fn outermost_drops_what_is_inside_another_selected_block() {
        let items = outline(TRIP);
        assert_eq!(outermost(&items, &[3, 0, 2, 5]), [0, 5]);
        assert_eq!(outermost(&items, &[3, 4, 1]), [1, 3, 4]);
        assert_eq!(outermost(&items, &[2, 3]), [2]);
    }
}
