//! The scene hierarchy as edited in the explorer: which node is the child of which, and in what order.
//!
//! The composed mesh keeps its nodes in a fixed order (model nodes, environment, the primitives
//! collection, added objects); that order is what the per-object data and the edit lists are indexed
//! by. The tree the user arranges is therefore stored on top of it, as a list of `(name, parent name)`
//! pairs in display order (`""` = the scene root, `Edits::hier`), and applied to each node's `parent`.
use crate::obj::{Kind, Node};
use std::collections::HashMap;

/// Where a dragged node lands relative to the row it is dropped on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// Next to the target, before it.
    Before(usize),
    /// Next to the target, after it.
    After(usize),
    /// As the last child of the target.
    Inside(usize),
    /// As the first child of the target.
    First(usize),
}

/// The parent a node has without any rearranging.
fn default_parent(base: &[Node], i: usize) -> Option<usize> {
    let nb = base.len();
    if i < nb {
        base[i].parent
    } else if i == nb || i == nb + 1 {
        Some(0) // the environment and the primitives collection sit under the scene
    } else {
        Some(nb + 1) // added objects start inside the primitives collection
    }
}

/// True when `anc` is `i` or one of its ancestors.
pub fn is_ancestor_or_self(nodes: &[Node], anc: usize, mut i: usize) -> bool {
    for _ in 0..=nodes.len() {
        if i == anc {
            return true;
        }
        match nodes[i].parent {
            Some(p) => i = p,
            None => return false,
        }
    }
    true // a cycle: treat as related, so nothing is ever moved into it
}

/// Gives every node its effective parent: the model's own, then the user's arrangement on top.
pub fn apply(nodes: &mut [Node], base: &[Node], hier: &[(String, String)]) {
    for i in 0..nodes.len() {
        nodes[i].parent = default_parent(base, i);
    }
    if hier.is_empty() {
        return;
    }
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate().rev() {
        index.insert(n.name.clone(), i); // the first node of a name wins
    }
    let fixed = base.len() + 1; // the primitives collection never moves
    for (child, parent) in hier {
        let Some(&c) = index.get(child.as_str()) else { continue };
        let p = if parent.is_empty() { Some(0) } else { index.get(parent.as_str()).copied() };
        let Some(p) = p else { continue };
        if c == 0 || c == fixed || nodes[c].kind == Kind::Environment || nodes[p].kind == Kind::Environment || nodes[c].kind == Kind::Material || nodes[p].kind == Kind::Material || is_ancestor_or_self(nodes, c, p) {
            continue;
        }
        nodes[c].parent = Some(p);
    }
}

/// Children of every node, siblings in the user's order (unlisted nodes last, in index order).
pub fn children(nodes: &[Node], hier: &[(String, String)]) -> Vec<Vec<usize>> {
    let rank: HashMap<&str, usize> = hier.iter().enumerate().map(|(i, (n, _))| (n.as_str(), i)).collect();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        if let Some(p) = n.parent {
            kids[p].push(i);
        }
    }
    for k in &mut kids {
        k.sort_by_key(|&i| (rank.get(nodes[i].name.as_str()).copied().unwrap_or(usize::MAX), i));
    }
    kids
}

/// Every node in display order (parents before their children).
pub fn order(nodes: &[Node], hier: &[(String, String)]) -> Vec<usize> {
    let kids = children(nodes, hier);
    let mut out = Vec::with_capacity(nodes.len());
    let mut stack = vec![0usize];
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(kids[n].iter().rev());
    }
    out
}

/// All nodes below `i` (children, their children ...), not including `i`.
pub fn descendants(nodes: &[Node], i: usize) -> Vec<usize> {
    (0..nodes.len()).filter(|&j| j != i && is_ancestor_or_self(nodes, i, j)).collect()
}

/// The arrangement after dragging `node` to `place`, or None when that move is not allowed (into itself
/// or its own children, moving the scene / environment, nothing changes ...). `nodes` carry the current
/// effective parents.
pub fn moved(nodes: &[Node], hier: &[(String, String)], node: usize, place: Place) -> Option<Vec<(String, String)>> {
    if node == 0 || nodes[node].kind == Kind::Environment || nodes[node].kind == Kind::Material {
        return None; // a material belongs to its object and cannot be moved
    }
    let mut kids = children(nodes, hier);
    let old_parent = nodes[node].parent?;
    kids[old_parent].retain(|&k| k != node);
    let (np, idx) = match place {
        Place::Before(t) | Place::After(t) => {
            let p = nodes[t].parent?; // the scene root has no siblings
            let at = kids[p].iter().position(|&k| k == t)?;
            (p, if matches!(place, Place::Before(_)) { at } else { at + 1 })
        }
        Place::Inside(t) => (t, kids[t].len()),
        Place::First(t) => (t, 0),
    };
    if nodes[np].kind == Kind::Environment || nodes[np].kind == Kind::Material || is_ancestor_or_self(nodes, node, np) {
        return None;
    }
    kids[np].insert(idx, node);
    let mut out = Vec::with_capacity(nodes.len());
    let mut stack: Vec<usize> = kids[0].iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        let parent = if n == node { np } else { nodes[n].parent.unwrap_or(0) };
        out.push((nodes[n].name.clone(), if parent == 0 { String::new() } else { nodes[parent].name.clone() }));
        stack.extend(kids[n].iter().rev());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 0 Scene { 1 Rocks { 2 A, 3 B }, 4 C, 5 Env }
    fn sample() -> Vec<Node> {
        vec![
            Node::new(Kind::Collection, None, "Scene", false),
            Node::new(Kind::Collection, Some(0), "Rocks", false),
            Node::new(Kind::Object, Some(1), "A", false),
            Node::new(Kind::Object, Some(1), "B", false),
            Node::new(Kind::Object, Some(0), "C", false),
            Node::new(Kind::Environment, Some(0), "Environment", false),
        ]
    }

    fn arranged(base: &[Node], hier: &[(String, String)]) -> Vec<Node> {
        let mut n = base.to_vec();
        apply(&mut n, base, hier);
        n
    }

    fn names(nodes: &[Node], ord: &[usize]) -> Vec<String> {
        ord.iter().map(|&i| nodes[i].name.clone()).collect()
    }

    #[test]
    fn default_order_is_the_model_order() {
        let base = sample();
        let n = arranged(&base, &[]);
        assert_eq!(names(&n, &order(&n, &[])), ["Scene", "Rocks", "A", "B", "C", "Environment"]);
    }

    #[test]
    fn an_object_can_become_the_child_of_another_object() {
        let base = sample();
        let n = arranged(&base, &[]);
        let h = moved(&n, &[], 4, Place::Inside(2)).unwrap(); // C inside A
        let n2 = arranged(&base, &h);
        assert_eq!(n2[4].parent, Some(2));
        assert_eq!(names(&n2, &order(&n2, &h)), ["Scene", "Rocks", "A", "C", "B", "Environment"]);
        assert_eq!(descendants(&n2, 1), vec![2, 3, 4]);
    }

    #[test]
    fn dropping_above_and_below_reorders_siblings() {
        let base = sample();
        let n = arranged(&base, &[]);
        let h = moved(&n, &[], 3, Place::Before(2)).unwrap(); // B above A
        let n2 = arranged(&base, &h);
        assert_eq!(names(&n2, &order(&n2, &h)), ["Scene", "Rocks", "B", "A", "C", "Environment"]);
        let h2 = moved(&n2, &h, 2, Place::After(4)).unwrap(); // A below C, out of Rocks
        let n3 = arranged(&base, &h2);
        assert_eq!(n3[2].parent, Some(0));
        assert_eq!(names(&n3, &order(&n3, &h2)), ["Scene", "Rocks", "B", "C", "A", "Environment"]);
    }

    #[test]
    fn cycles_and_fixed_nodes_are_refused() {
        let base = sample();
        let n = arranged(&base, &[]);
        assert!(moved(&n, &[], 1, Place::Inside(2)).is_none(), "a node cannot go into its own child");
        assert!(moved(&n, &[], 1, Place::Inside(1)).is_none());
        assert!(moved(&n, &[], 0, Place::Inside(4)).is_none(), "the scene stays");
        assert!(moved(&n, &[], 5, Place::Inside(1)).is_none(), "the environment stays");
        assert!(moved(&n, &[], 4, Place::Inside(5)).is_none(), "nothing goes into the environment");
    }

    #[test]
    fn a_stale_arrangement_is_ignored_safely() {
        let base = sample();
        let h = vec![("Gone".to_string(), "A".to_string()), ("A".to_string(), "Missing".to_string()), ("A".to_string(), "A".to_string())];
        let n = arranged(&base, &h);
        assert_eq!(n[2].parent, Some(1));
    }
}
