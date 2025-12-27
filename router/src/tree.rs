use crate::escape::{UnescapedRef, UnescapedRoute};
use crate::{params::Params, InsertError, MatchError};

use std::cmp::min;
use std::collections::VecDeque;
use std::ops::Range;
use std::{fmt, mem};

/// A radix tree used for URL path matching.
///
/// See [the crate documentation](crate) for details.
#[derive(Clone)]
pub struct Node<T> {
    // This node's prefix.
    pub(crate) prefix: UnescapedRoute,

    // The priority of this node.
    //
    // Nodes with more children are higher priority and searched first.
    pub(crate) priority: u32,

    // Whether this node contains a wildcard child.
    pub(crate) wild_child: bool,

    // The first character of any static children, for fast linear search.
    pub(crate) indices: Vec<u8>,

    // The type of this node.
    pub(crate) node_type: NodeType,

    // The children of this node.
    pub(crate) children: Vec<Node<T>>,

    // The value stored at this node.
    //
    // See `Node::at` for why an `UnsafeCell` is necessary.
    pub(crate) value: Option<T>,

    // A parameter name remapping, stored at nodes that hold values.
    pub(crate) remapping: ParamRemapping,
}

/// The types of nodes a tree can hold.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone)]
pub(crate) enum NodeType {
    /// The root path.
    Root,

    /// A route parameter, e.g. '/{id}'.
    ///
    /// If `suffix` is `false`, the only child of this node is
    /// a static '/', allowing for a fast path when searching.
    /// Otherwise, the route may have static suffixes, e.g. '/{id}.png'.
    ///
    /// The leaves of a parameter node are the static suffixes
    /// sorted by length. This allows for a reverse linear search
    /// to determine the correct leaf. It would also be possible to
    /// use a reverse prefix-tree here, but is likely not worth the
    /// complexity.
    Param { suffix: bool },

    /// A catch-all parameter, e.g. '/{*file}'.
    CatchAll,

    /// A static prefix, e.g. '/foo'.
    Static,
}

/// Tracks the current node and its parent during insertion.
struct InsertState<'node, T> {
    parent: &'node mut Node<T>,
    child: Option<usize>,
}

impl<'node, T> InsertState<'node, T> {
    /// Returns a reference to the parent node for the traversal.
    fn parent(&self) -> Option<&Node<T>> {
        match self.child {
            None => None,
            Some(_) => Some(self.parent),
        }
    }

    /// Returns a reference to the current node in the traversal.
    fn node(&self) -> &Node<T> {
        match self.child {
            None => self.parent,
            Some(i) => &self.parent.children[i],
        }
    }

    /// Returns a mutable reference to the current node in the traversal.
    fn node_mut(&mut self) -> &mut Node<T> {
        match self.child {
            None => self.parent,
            Some(i) => &mut self.parent.children[i],
        }
    }

    /// Move the current node to its i'th child.
    fn set_child(self, i: usize) -> InsertState<'node, T> {
        match self.child {
            None => InsertState {
                parent: self.parent,
                child: Some(i),
            },
            Some(prev) => InsertState {
                parent: &mut self.parent.children[prev],
                child: Some(i),
            },
        }
    }
}

impl<T> Node<T> {
    // Insert a route into the tree.
    pub fn insert(&mut self, route: String, val: T) -> Result<(), InsertError> {
        let route = UnescapedRoute::new(route.into_bytes());
        let (route, remapping) = normalize_params(route)?;
        let mut remaining = route.as_ref();

        self.priority += 1;

        // If the tree is empty, insert the root node.
        if self.value.is_none() && self.children.is_empty() {
            let last = self.insert_route(remaining, val)?;
            last.remapping = remapping;
            self.node_type = NodeType::Root;
            return Ok(());
        }

        let mut state = InsertState {
            parent: self,
            child: None,
        };

        'walk: loop {
            // Find the common prefix between the route and the current node.
            let len = min(remaining.len(), state.node().prefix.len());
            let common_prefix = (0..len)
                .find(|&i| {
                    remaining[i] != state.node().prefix[i]
                    // Make sure not confuse the start of a wildcard with an escaped `{`.
                        || remaining.is_escaped(i) != state.node().prefix.is_escaped(i)
                })
                .unwrap_or(len);

            // If this node has a longer prefix than we need, we have to fork and extract the
            // common prefix into a shared parent.
            if state.node().prefix.len() > common_prefix {
                let node = state.node_mut();

                // Move the non-matching suffix into a child node.
                let suffix = node.prefix.as_ref().slice_off(common_prefix).to_owned();
                let child = Node {
                    prefix: suffix,
                    value: node.value.take(),
                    indices: node.indices.clone(),
                    wild_child: node.wild_child,
                    children: mem::take(&mut node.children),
                    remapping: mem::take(&mut node.remapping),
                    priority: node.priority - 1,
                    node_type: NodeType::Static,
                };

                // The current node now only holds the common prefix.
                node.children = vec![child];
                node.indices = vec![node.prefix[common_prefix]];
                node.prefix = node.prefix.as_ref().slice_until(common_prefix).to_owned();
                node.wild_child = false;
                continue;
            }

            if remaining.len() == common_prefix {
                let node = state.node_mut();

                // This node must not already contain a value.
                if node.value.is_some() {
                    return Err(InsertError::conflict(&route, remaining, node));
                }

                // Insert the value.
                node.value = Some(val);
                node.remapping = remapping;
                return Ok(());
            }

            let common_remaining = remaining;

            // Otherwise, the route has a remaining non-matching suffix.
            //
            // We have to search deeper.
            remaining = remaining.slice_off(common_prefix);
            let next = remaining[0];

            // For parameters with a suffix, we have to find the matching suffix or create a new child node.
            if let NodeType::Param { suffix: has_suffix } = state.node().node_type {
                let terminator = remaining
                    .iter()
                    .position(|&b| b == b'/')
                    .map(|b| b + 1)
                    .unwrap_or(remaining.len());

                let suffix = remaining.slice_until(terminator);

                let mut extra_trailing_slash = false;
                for (i, child) in state.node().children.iter().enumerate() {
                    // Find a matching suffix.
                    if *child.prefix == **suffix {
                        state = state.set_child(i);
                        state.node_mut().priority += 1;
                        continue 'walk;
                    }

                    // The suffix matches except for an extra trailing slash.
                    if child.prefix.len() <= suffix.len() {
                        let (common, remaining) = suffix.split_at(child.prefix.len());
                        if *common == *child.prefix && remaining == *b"/" {
                            extra_trailing_slash = true;
                        }
                    }
                }

                // If we are inserting a conflicting suffix, and there is a static prefix that
                // already leads to this route parameter, we have a prefix-suffix conflict.
                if !extra_trailing_slash && !matches!(*suffix, b"" | b"/") {
                    if let Some(parent) = state.parent() {
                        if parent.prefix_wild_child_in_segment() {
                            return Err(InsertError::conflict(&route, common_remaining, parent));
                        }
                    }
                }

                // Multiple parameters within the same segment, e.g. `/{foo}{bar}`.
                if matches!(find_wildcard(suffix), Ok(Some(_))) {
                    return Err(InsertError::InvalidParamSegment);
                }

                // If there is no matching suffix, create a new suffix node.
                let child = state.node_mut().add_suffix_child(Node {
                    prefix: suffix.to_owned(),
                    node_type: NodeType::Static,
                    priority: 1,
                    ..Node::default()
                });

                let has_suffix = has_suffix || !matches!(*suffix, b"" | b"/");
                state.node_mut().node_type = NodeType::Param { suffix: has_suffix };

                state = state.set_child(child);

                // If this is the final route segment, insert the value.
                if terminator == remaining.len() {
                    state.node_mut().value = Some(val);
                    state.node_mut().remapping = remapping;
                    return Ok(());
                }

                // Otherwise, the previous node will hold only the suffix and we
                // need to create a new child for the remaining route.
                remaining = remaining.slice_off(terminator);

                // Create a static node unless we are inserting a parameter.
                if remaining[0] != b'{' || remaining.is_escaped(0) {
                    let child = state.node_mut().add_child(Node {
                        node_type: NodeType::Static,
                        priority: 1,
                        ..Node::default()
                    });
                    state.node_mut().indices.push(remaining[0]);
                    state = state.set_child(child);
                }

                // Insert the remaining route.
                let last = state.node_mut().insert_route(remaining, val)?;
                last.remapping = remapping;
                return Ok(());
            }

            // Find a child node that matches the next character in the route.
            for mut i in 0..state.node().indices.len() {
                if next == state.node().indices[i] {
                    // Make sure not confuse the start of a wildcard with an escaped `{` or `}`.
                    if matches!(next, b'{' | b'}') && !remaining.is_escaped(0) {
                        continue;
                    }

                    // Continue searching in the child.
                    i = state.node_mut().update_child_priority(i);
                    state = state.set_child(i);
                    continue 'walk;
                }
            }

            // We couldn't find a matching child.
            //
            // If we're not inserting a wildcard we have to create a static child.
            if (next != b'{' || remaining.is_escaped(0)) && state.node().node_type != NodeType::CatchAll {
                let node = state.node_mut();

                let terminator = remaining.iter().position(|&b| b == b'/').unwrap_or(remaining.len());

                if let Ok(Some(wildcard)) = find_wildcard(remaining.slice_until(terminator)) {
                    // If we are inserting a parameter prefix and this node already has a parameter suffix,
                    // we have a prefix-suffix conflict.
                    if wildcard.start > 0 && node.suffix_wild_child_in_segment() {
                        return Err(InsertError::conflict(&route, remaining, node));
                    }

                    // Similarly, we are inserting a parameter suffix and this node already has a parameter
                    // prefix, we have a prefix-suffix conflict.
                    let suffix = remaining.slice_off(wildcard.end);
                    if !matches!(*suffix, b"" | b"/") && node.prefix_wild_child_in_segment() {
                        return Err(InsertError::conflict(&route, remaining, node));
                    }
                }

                node.indices.push(next);
                let child = node.add_child(Node::default());
                let child = node.update_child_priority(child);

                // Insert into the newly created node.
                let last = node.children[child].insert_route(remaining, val)?;
                last.remapping = remapping;
                return Ok(());
            }

            // We're trying to insert a wildcard.
            //
            // If this node already has a wildcard child, we have to make sure it matches.
            if state.node().wild_child {
                // Wildcards are always the last child.
                let wild_child = state.node().children.len() - 1;
                state = state.set_child(wild_child);
                state.node_mut().priority += 1;

                // Make sure the route parameter matches.
                if let Some(wildcard) = remaining.get(..state.node().prefix.len()) {
                    if *wildcard != *state.node().prefix {
                        return Err(InsertError::conflict(&route, remaining, state.node()));
                    }
                }

                // Catch-all routes cannot have children.
                if state.node().node_type == NodeType::CatchAll {
                    return Err(InsertError::conflict(&route, remaining, state.node()));
                }

                if let Some(parent) = state.parent() {
                    // If there is a route with both a prefix and a suffix, and we are inserting a route with
                    // a matching prefix but _without_ a suffix, we have a prefix-suffix conflict.
                    if !parent.prefix.ends_with(b"/")
                        && matches!(state.node().node_type, NodeType::Param { suffix: true })
                    {
                        let terminator = remaining
                            .iter()
                            .position(|&b| b == b'/')
                            .map(|b| b + 1)
                            .unwrap_or(remaining.len());

                        if let Ok(Some(wildcard)) = find_wildcard(remaining.slice_until(terminator)) {
                            let suffix = remaining.slice_off(wildcard.end);
                            if matches!(*suffix, b"" | b"/") {
                                return Err(InsertError::conflict(&route, remaining, parent));
                            }
                        }
                    }
                }

                // Continue with the wildcard node.
                continue 'walk;
            }

            if let Ok(Some(wildcard)) = find_wildcard(remaining) {
                let node = state.node();
                let suffix = remaining.slice_off(wildcard.end);

                // If we are inserting a suffix and there is a static prefix that already leads to this
                // route parameter, we have a prefix-suffix conflict.
                if !matches!(*suffix, b"" | b"/") && node.prefix_wild_child_in_segment() {
                    return Err(InsertError::conflict(&route, remaining, node));
                }

                // Similarly, if we are inserting a longer prefix, and there is a route that leads to this
                // parameter that includes a suffix, we have a prefix-suffix conflict.
                if let Some(i) = common_prefix.checked_sub(1) {
                    if common_remaining[i] != b'/' && node.suffix_wild_child_in_segment() {
                        return Err(InsertError::conflict(&route, remaining, node));
                    }
                }
            }

            // Otherwise, create a new node for the wildcard and insert the route.
            let last = state.node_mut().insert_route(remaining, val)?;
            last.remapping = remapping;
            return Ok(());
        }
    }

    /// Returns `true` if there is a wildcard node that contains a prefix within the current route segment,
    /// i.e. before the next trailing slash
    fn prefix_wild_child_in_segment(&self) -> bool {
        if matches!(self.node_type, NodeType::Root) && self.prefix.is_empty() {
            return false;
        }

        if self.prefix.ends_with(b"/") {
            self.children.iter().any(Node::prefix_wild_child_in_segment)
        } else {
            self.children.iter().any(Node::wild_child_in_segment)
        }
    }

    /// Returns `true` if there is a wildcard node within the current route segment, i.e. before the
    /// next trailing slash.
    fn wild_child_in_segment(&self) -> bool {
        if self.prefix.contains(&b'/') {
            return false;
        }

        if matches!(self.node_type, NodeType::Param { .. }) {
            return true;
        }

        self.children.iter().any(Node::wild_child_in_segment)
    }

    /// Returns `true` if there is a wildcard parameter node that contains a suffix within the current route
    /// segment, i.e. before a trailing slash.
    fn suffix_wild_child_in_segment(&self) -> bool {
        if matches!(self.node_type, NodeType::Param { suffix: true }) {
            return true;
        }

        self.children.iter().any(|child| {
            if child.prefix.contains(&b'/') {
                return false;
            }

            child.suffix_wild_child_in_segment()
        })
    }

    // Insert a route at this node.
    //
    // If the route starts with a wildcard, a child node will be created for the parameter
    // and `wild_child` will be set on the parent.
    fn insert_route(&mut self, mut prefix: UnescapedRef<'_>, val: T) -> Result<&mut Node<T>, InsertError> {
        let mut node = self;

        loop {
            // Search for a wildcard segment.
            let Some(wildcard) = find_wildcard(prefix)? else {
                // There is no wildcard, simply insert into the current node.
                node.value = Some(val);
                node.prefix = prefix.to_owned();
                return Ok(node);
            };

            // Inserting a catch-all route.
            if prefix[wildcard.clone()][1] == b'*' {
                // Ensure there is no suffix after the parameter, e.g. `/foo/{*x}/bar`.
                if wildcard.end != prefix.len() {
                    return Err(InsertError::InvalidCatchAll);
                }

                // Add the prefix before the wildcard into the current node.
                if wildcard.start > 0 {
                    node.prefix = prefix.slice_until(wildcard.start).to_owned();
                    prefix = prefix.slice_off(wildcard.start);
                }

                // Add the catch-all as a child node.
                let child = node.add_child(Node {
                    prefix: prefix.to_owned(),
                    node_type: NodeType::CatchAll,
                    value: Some(val),
                    priority: 1,
                    ..Node::default()
                });
                node.wild_child = true;
                return Ok(&mut node.children[child]);
            }

            // Otherwise, we're inserting a regular route parameter.
            //
            // Add the prefix before the wildcard into the current node.
            if wildcard.start > 0 {
                node.prefix = prefix.slice_until(wildcard.start).to_owned();
                prefix = prefix.slice_off(wildcard.start);
            }

            // Find the end of this route segment.
            let terminator = prefix
                .iter()
                .position(|&b| b == b'/')
                .map(|b| b + 1)
                .unwrap_or(prefix.len());

            let wildcard = prefix.slice_until(wildcard.len());
            let suffix = prefix.slice_until(terminator).slice_off(wildcard.len());
            prefix = prefix.slice_off(terminator);

            // Multiple parameters within the same segment, e.g. `/{foo}{bar}`.
            if matches!(find_wildcard(suffix), Ok(Some(_))) {
                return Err(InsertError::InvalidParamSegment);
            }

            // Add the parameter as a child node.
            let has_suffix = !matches!(*suffix, b"" | b"/");
            let child = node.add_child(Node {
                priority: 1,
                node_type: NodeType::Param { suffix: has_suffix },
                prefix: wildcard.to_owned(),
                ..Node::default()
            });

            node.wild_child = true;
            node = &mut node.children[child];

            // Add the static suffix until the '/', if there is one.
            //
            // Note that for '/' suffixes where `suffix: false`, this
            // unconditionally introduces an extra node for the '/'
            // without attempting to merge with the remaining route.
            // This makes converting a non-suffix parameter node into
            // a suffix one easier during insertion, but slightly hurts
            // performance.
            if !suffix.is_empty() {
                let child = node.add_suffix_child(Node {
                    priority: 1,
                    node_type: NodeType::Static,
                    prefix: suffix.to_owned(),
                    ..Node::default()
                });

                node = &mut node.children[child];
            }

            // If the route ends here, insert the value.
            if prefix.is_empty() {
                node.value = Some(val);
                return Ok(node);
            }

            // If there is a static segment after the '/', setup the node
            // for the rest of the route.
            if prefix[0] != b'{' || prefix.is_escaped(0) {
                node.indices.push(prefix[0]);
                let child = node.add_child(Node {
                    priority: 1,
                    ..Node::default()
                });
                node = &mut node.children[child];
            }
        }
    }

    // Adds a child to this node, keeping wildcards at the end.
    fn add_child(&mut self, child: Node<T>) -> usize {
        let len = self.children.len();

        if self.wild_child && len > 0 {
            self.children.insert(len - 1, child);
            len - 1
        } else {
            self.children.push(child);
            len
        }
    }

    // Adds a suffix child to this node, keeping suffixes sorted by ascending length.
    fn add_suffix_child(&mut self, child: Node<T>) -> usize {
        let i = self
            .children
            .partition_point(|node| node.prefix.len() >= child.prefix.len());
        self.children.insert(i, child);
        i
    }

    // Increments priority of the given child node, reordering the children if necessary.
    //
    // Returns the new index of the node.
    fn update_child_priority(&mut self, i: usize) -> usize {
        self.children[i].priority += 1;
        let priority = self.children[i].priority;

        // Move the node to the front as necessary.
        let mut updated = i;
        while updated > 0 && self.children[updated - 1].priority < priority {
            self.children.swap(updated - 1, updated);
            updated -= 1;
        }

        // Update the position of the indices to match.
        if updated != i {
            self.indices[updated..=i].rotate_right(1);
        }

        updated
    }

    /// Removes a route from the tree, returning the value if the route already existed.
    ///
    /// The provided path should be the same as the one used to insert the route, including
    /// wildcards.
    pub fn remove(&mut self, route: String) -> Option<T> {
        let route = UnescapedRoute::new(route.into_bytes());
        let (route, remapping) = normalize_params(route).ok()?;
        let mut remaining = route.unescaped();

        // Check if we are removing the root node.
        if remaining == self.prefix.unescaped() {
            let value = self.value.take();

            // If the root node has no children, we can reset it.
            if self.children.is_empty() {
                *self = Node::default();
            }

            return value;
        }

        let mut node = self;
        'walk: loop {
            // Could not find a match.
            if remaining.len() <= node.prefix.len() {
                return None;
            }

            // Otherwise, the path is longer than this node's prefix, search deeper.
            let (prefix, rest) = remaining.split_at(node.prefix.len());

            // The prefix does not match.
            if prefix != node.prefix.unescaped() {
                return None;
            }

            let next = rest[0];
            remaining = rest;

            // If this is a parameter node, we have to find the matching suffix.
            if matches!(node.node_type, NodeType::Param { .. }) {
                let terminator = remaining
                    .iter()
                    .position(|&b| b == b'/')
                    .map(|b| b + 1)
                    .unwrap_or(remaining.len());

                let suffix = &remaining[..terminator];

                for (i, child) in node.children.iter().enumerate() {
                    // Find the matching suffix.
                    if *child.prefix == *suffix {
                        // If this is the end of the path, remove the suffix node.
                        if terminator == remaining.len() {
                            return node.remove_child(i, &remapping);
                        }

                        // Otherwise, continue searching.
                        remaining = &remaining[terminator - child.prefix.len()..];
                        node = &mut node.children[i];
                        continue 'walk;
                    }
                }
            }

            // Find a child node that matches the next character in the route.
            if let Some(i) = node.indices.iter().position(|&c| c == next) {
                // The route matches, remove the node.
                if node.children[i].prefix.unescaped() == remaining {
                    return node.remove_child(i, &remapping);
                }

                // Otherwise, continue searching.
                node = &mut node.children[i];
                continue 'walk;
            }

            // If there is no matching wildcard child, there is no matching route.
            if !node.wild_child {
                return None;
            }

            // If the route does match, remove the node.
            if node.children.last_mut().unwrap().prefix.unescaped() == remaining {
                return node.remove_child(node.children.len() - 1, &remapping);
            }

            // Otherwise, keep searching deeper.
            node = node.children.last_mut().unwrap();
        }
    }

    /// Remove the child node at the given index, if the route parameters match.
    fn remove_child(&mut self, i: usize, remapping: &ParamRemapping) -> Option<T> {
        // Require an exact match to remove a route.
        //
        // For example, `/{a}` cannot be used to remove `/{b}`.
        if self.children[i].remapping != *remapping {
            return None;
        }

        // If the node does not have any children, we can remove it completely.
        if self.children[i].children.is_empty() {
            // Remove the child node.
            let child = self.children.remove(i);

            match child.node_type {
                // Remove the index if we removed a static prefix that is
                // not a suffix node.
                NodeType::Static if !matches!(self.node_type, NodeType::Param { .. }) => {
                    self.indices.remove(i);
                }

                // Otherwise, we removed a wildcard.
                _ => self.wild_child = false,
            }

            child.value
        }
        // Otherwise, remove the value but preserve the node.
        else {
            self.children[i].value.take()
        }
    }

    /// Iterates over the tree and calls the given visitor function
    /// with fully resolved path and its value.
    pub fn for_each<V: FnMut(String, T)>(self, mut visitor: V) {
        let mut queue = VecDeque::from([(self.prefix.clone(), self)]);

        // Perform a BFS on the routing tree.
        while let Some((mut prefix, mut node)) = queue.pop_front() {
            denormalize_params(&mut prefix, &node.remapping);

            if let Some(value) = node.value.take() {
                let path = String::from_utf8(prefix.unescaped().to_vec()).unwrap();
                visitor(path, value);
            }

            // Traverse the child nodes.
            for child in node.children {
                let mut prefix = prefix.clone();
                prefix.append(&child.prefix);
                queue.push_back((prefix, child));
            }
        }
    }
}

/// A wildcard node that was skipped during a tree search.
///
/// Contains the state necessary to backtrack to the given node.
struct Skipped<'node, 'path, T> {
    // The node that was skipped.
    node: &'node Node<T>,

    /// The path at the time we skipped this node.
    path: &'path str,

    // The number of parameters that were present.
    params: usize,
}

impl<T> Node<T> {
    // Returns the node matching the given path.
    //
    // Returning an `UnsafeCell` allows us to avoid duplicating the logic between `Node::at` and
    // `Node::at_mut`, as Rust doesn't have a great way of abstracting over mutability.
    #[inline]
    pub fn at<'s>(&'s self, mut path: &str) -> Result<(&'s T, Params), MatchError> {
        let mut node = self;
        let mut backtracking = false;
        let mut params = Params::new();
        let mut skipped: Vec<Skipped<'_, '_, T>> = Vec::new();

        'backtrack: loop {
            'walk: loop {
                // Reached the end of the
                if path.len() <= node.prefix.len() {
                    // Check for an exact match.
                    if path.as_bytes() == &*node.prefix {
                        // Found the matching value.
                        if let Some(ref value) = node.value {
                            // Remap the keys of any route parameters we accumulated during the search.
                            params.for_each_key_mut(|(i, param)| *param = node.remapping[i].as_str().into());
                            return Ok((value, params));
                        }
                    }

                    break 'walk;
                }

                // Otherwise, the path is longer than this node's prefix, search deeper.
                let (prefix, rest) = path.split_at(node.prefix.len());

                // The prefix does not match.
                if prefix.as_bytes() != &*node.prefix {
                    break 'walk;
                }

                let previous = path;
                path = rest;

                // If we are currently backtracking, avoid searching static children
                // that we already searched.
                if !backtracking {
                    let next = path.as_bytes()[0];

                    // Find a child node that matches the next character in the path.
                    if let Some(i) = node.indices.iter().position(|&c| c == next) {
                        // Keep track of wildcard routes that we skip.
                        //
                        // We may end up needing to backtrack later in case we do not find a
                        // match.
                        if node.wild_child {
                            skipped.push(Skipped {
                                node,
                                path: previous,
                                params: params.len(),
                            });
                        }

                        // Continue searching.
                        node = &node.children[i];
                        continue 'walk;
                    }
                }

                // We didn't find a matching static child.
                //
                // If there are no wildcards, then there are no matching routes in the tree.
                if !node.wild_child {
                    break 'walk;
                }

                // Continue searching in the wildcard child, which is kept at the end of the list.
                node = node.children.last().unwrap();
                match node.node_type {
                    NodeType::Param { suffix: false } => {
                        // Check for more path segments.
                        let terminator = match path.as_bytes().iter().position(|&c| c == b'/') {
                            // Double `//` implying an empty parameter, no match.
                            Some(0) => break 'walk,

                            // Found another segment.
                            Some(i) => i,

                            // This is the last path segment.
                            None => {
                                // If this is the last path segment and there is a matching
                                // value without a suffix, we have a match.
                                let Some(ref value) = node.value else {
                                    break 'walk;
                                };

                                // Store the parameter value.
                                params.push("", path);

                                // Remap the keys of any route parameters we accumulated during the search.
                                params.for_each_key_mut(|(i, param)| *param = node.remapping[i].as_str().into());

                                return Ok((value, params));
                            }
                        };

                        // Found another path segment.
                        let (param, rest) = path.split_at(terminator);

                        // If there is a static child, continue the search.
                        let [child] = node.children.as_slice() else {
                            break 'walk;
                        };

                        // Store the parameter value.
                        // Parameters are normalized so this key is irrelevant for now.
                        params.push("", param);

                        // Continue searching.
                        path = rest;
                        node = child;
                        backtracking = false;
                        continue 'walk;
                    }

                    NodeType::Param { suffix: true } => {
                        // Check for more path segments.
                        let slash = path.as_bytes().iter().position(|&c| c == b'/');
                        let terminator = match slash {
                            // Double `//` implying an empty parameter, no match.
                            Some(0) => break 'walk,

                            // Found another segment.
                            Some(i) => i + 1,

                            // This is the last path segment.
                            None => path.len(),
                        };

                        for child in node.children.iter() {
                            // Ensure there is a possible match with a non-zero suffix.
                            if child.prefix.len() >= terminator {
                                continue;
                            }

                            let suffix_start = terminator - child.prefix.len();
                            let (param, suffix) = path[..terminator].split_at(suffix_start);

                            // Continue searching if the suffix matches.
                            if suffix.as_bytes() == &*child.prefix {
                                node = child;
                                path = &path[suffix_start..];
                                backtracking = false;
                                // Parameters are normalized so this key is irrelevant for now.
                                params.push("", param);
                                continue 'walk;
                            }
                        }

                        // If this is the last path segment and there is a matching
                        // value without a suffix, we have a match.
                        let value = match node.value {
                            // Found the matching value.
                            Some(ref value) if slash.is_none() => value,
                            _ => break 'walk,
                        };

                        // Store the parameter value.
                        params.push("", path);

                        // Remap the keys of any route parameters we accumulated during the search.
                        params.for_each_key_mut(|(i, param)| *param = node.remapping[i].as_str().into());

                        return Ok((value, params));
                    }

                    NodeType::CatchAll => {
                        // Catch-all segments are only allowed at the end of the route, meaning
                        // this node must contain the value.
                        let value = match node.value {
                            // Found the matching value.
                            Some(ref value) => value,

                            // Otherwise, there are no matching routes in the tree.
                            None => return Err(MatchError),
                        };

                        // Remap the keys of any route parameters we accumulated during the search.
                        params.for_each_key_mut(|(i, param)| *param = node.remapping[i].as_str().into());

                        // Store the final catch-all parameter (`{*...}`).
                        let key = &node.prefix[2..node.prefix.len() - 1];

                        params.push(std::str::from_utf8(key).unwrap(), path);

                        return Ok((value, params));
                    }

                    _ => unreachable!(),
                }
            }

            // Try backtracking to any matching wildcard nodes that we skipped while
            // traversing the tree.
            while let Some(skipped) = skipped.pop() {
                if skipped.path.ends_with(path) {
                    // Found a matching node, restore the search state.
                    path = skipped.path;
                    node = skipped.node;
                    backtracking = true;
                    params.truncate(skipped.params);
                    continue 'backtrack;
                }
            }

            // handle empty wildcard node with lowest priority.
            if let Some(child) = node.children.first() {
                if child.prefix.unescaped() == b"{*}" {
                    if let Some(ref val) = child.value {
                        return Ok((val, params));
                    }
                }
            }

            return Err(MatchError);
        }
    }

    /// Test helper that ensures route priorities are consistent.
    pub(super) fn check_priorities(&self) -> Result<u32, (u32, u32)> {
        let mut priority: u32 = 0;
        for child in &self.children {
            priority += child.check_priorities()?;
        }

        if self.value.is_some() {
            priority += 1;
        }

        if self.priority != priority {
            return Err((self.priority, priority));
        }

        Ok(priority)
    }
}

/// An ordered list of route parameters keys for a specific route.
///
/// To support conflicting routes like `/{a}/foo` and `/{b}/bar`, route parameters
/// are normalized before being inserted into the tree. Parameter remapping are
/// stored at nodes containing values, containing the "true" names of all route parameters
/// for the given route.
type ParamRemapping = Vec<String>;

/// Returns `path` with normalized route parameters, and a parameter remapping
/// to store at the node for this route.
///
/// Note that the parameter remapping may contain unescaped characters.
fn normalize_params(mut path: UnescapedRoute) -> Result<(UnescapedRoute, ParamRemapping), InsertError> {
    let mut start = 0;
    let mut original = ParamRemapping::new();

    // Parameter names are normalized alphabetically.
    let mut next = b'a';

    loop {
        // Find a wildcard to normalize.
        let mut wildcard = match find_wildcard(path.as_ref().slice_off(start))? {
            Some(wildcard) => wildcard,
            // No wildcard, we are done.
            None => return Ok((path, original)),
        };

        wildcard.start += start;
        wildcard.end += start;

        // Ensure the parameter has a valid name.
        if wildcard.len() < 2 {
            return Err(InsertError::InvalidParam);
        }

        // We don't need to normalize catch-all parameters, as they are always
        // at the end of a route.
        if path[wildcard.clone()][1] == b'*' {
            start = wildcard.end;
            continue;
        }

        // Normalize the parameter.
        let removed = path.splice(wildcard.clone(), vec![b'{', next, b'}']);

        // Preserve the original name for remapping.
        let mut removed = removed.skip(1).collect::<Vec<_>>();
        removed.pop();
        original.push(String::from_utf8(removed).unwrap());

        next += 1;
        if next > b'z' {
            panic!("Too many route parameters.");
        }

        // Continue the search after the parameter we just normalized.
        start = wildcard.start + 3;
    }
}

/// Restores `route` to it's original, denormalized form.
pub(crate) fn denormalize_params(route: &mut UnescapedRoute, params: &ParamRemapping) {
    let mut start = 0;
    let mut i = 0;

    loop {
        // Find a wildcard to denormalize.
        let mut wildcard = match find_wildcard(route.as_ref().slice_off(start)).unwrap() {
            Some(w) => w,
            None => return,
        };

        wildcard.start += start;
        wildcard.end += start;

        // Get the corresponding parameter remapping.
        let mut next = match params.get(i) {
            Some(param) => param.clone(),
            None => return,
        };

        // Denormalize this parameter.
        next.insert(0, '{');
        next.push('}');
        let _ = route.splice(wildcard.clone(), next.as_bytes().to_vec());

        i += 1;
        start = wildcard.start + next.len();
    }
}

// Searches for a wildcard segment and checks the path for invalid characters.
fn find_wildcard(path: UnescapedRef<'_>) -> Result<Option<Range<usize>>, InsertError> {
    for (start, &c) in path.iter().enumerate() {
        // Found an unescaped closing brace without a corresponding opening brace.
        if c == b'}' && !path.is_escaped(start) {
            return Err(InsertError::InvalidParam);
        }

        // Keep going until we find an unescaped opening brace.
        if c != b'{' || path.is_escaped(start) {
            continue;
        }

        // Ensure there is a non-empty parameter name.
        if path.get(start + 1) == Some(&b'}') {
            return Err(InsertError::InvalidParam);
        }

        // Find the corresponding closing brace.
        for (i, &c) in path.iter().enumerate().skip(start + 2) {
            match c {
                b'}' => {
                    // This closing brace was escaped, keep searching.
                    if path.is_escaped(i) {
                        continue;
                    }

                    // unlike matchit. single * is allowed as wildcard

                    // Ensure catch-all parameters have a non-empty name.
                    // if path.get(i - 1) == Some(&b'*') {
                    //     return Err(InsertError::InvalidParam);
                    // }

                    return Ok(Some(start..i + 1));
                }
                // `*` and `/` are invalid in parameter names.
                b'*' | b'/' => return Err(InsertError::InvalidParam),
                _ => {}
            }
        }

        // Missing closing brace.
        return Err(InsertError::InvalidParam);
    }

    Ok(None)
}

impl<T> Default for Node<T> {
    fn default() -> Node<T> {
        Node {
            remapping: ParamRemapping::new(),
            prefix: UnescapedRoute::default(),
            wild_child: false,
            node_type: NodeType::Static,
            indices: Vec::new(),
            children: Vec::new(),
            value: None,
            priority: 0,
        }
    }
}

impl<T> fmt::Debug for Node<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("Node");
        f.field("value", &self.value.as_ref())
            .field("prefix", &self.prefix)
            .field("node_type", &self.node_type)
            .field("children", &self.children);

        // Extra information for debugging purposes.
        #[cfg(test)]
        {
            let indices = self.indices.iter().map(|&x| char::from_u32(x as _)).collect::<Vec<_>>();

            let params = self.remapping.iter().collect::<Vec<_>>();

            f.field("indices", &indices).field("params", &params);
        }

        f.finish()
    }
}
