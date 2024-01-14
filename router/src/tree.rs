use core::{cmp::min, mem, str::from_utf8};

use super::{params::Params, InsertError, MatchError};

/// The types of nodes the tree can hold
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone)]
pub(crate) enum NodeType {
    /// The root path
    Root,
    /// A route parameter, ex: `/:id`.
    Param,
    /// A catchall parameter, ex: `/*file`
    CatchAll,
    /// Anything else
    Static,
}

/// A radix tree used for URL path matching.
///
/// See [the crate documentation](crate) for details.
#[derive(Clone)]
pub struct Node<T> {
    priority: u32,
    wild_child: bool,
    indices: Vec<u8>,
    value: Option<T>,
    pub(crate) param_remapping: ParamRemapping,
    pub(crate) node_type: NodeType,
    pub(crate) prefix: String,
    pub(crate) children: Vec<Self>,
}

impl<T> Default for Node<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Node<T> {
    pub const fn new() -> Self {
        Self {
            priority: 0,
            wild_child: false,
            indices: Vec::new(),
            value: None,
            param_remapping: ParamRemapping::new(),
            node_type: NodeType::Static,
            prefix: String::new(),
            children: Vec::new(),
        }
    }

    pub fn insert(&mut self, route: impl Into<String>, val: T) -> Result<(), InsertError> {
        let route = route.into().into_bytes();
        let (route, param_remapping) = normalize_params(route)?;
        let mut prefix = route.as_ref();

        self.priority += 1;

        // the tree is empty
        if self.prefix.is_empty() && self.children.is_empty() {
            let last = self.insert_child(prefix, &route, val)?;
            last.param_remapping = param_remapping;
            self.node_type = NodeType::Root;
            return Ok(());
        }

        let mut current = self;

        'walk: loop {
            // find the longest common prefix
            let len = min(prefix.len(), current.prefix.len());
            let common_prefix = (0..len)
                .find(|&i| prefix[i] != current.prefix.as_bytes()[i])
                .unwrap_or(len);

            // the common prefix is a substring of the current node's prefix, split the node
            if common_prefix < current.prefix.len() {
                let child = Node {
                    prefix: current.prefix[common_prefix..].into(),
                    children: mem::take(&mut current.children),
                    wild_child: current.wild_child,
                    indices: current.indices.clone(),
                    value: current.value.take(),
                    param_remapping: mem::take(&mut current.param_remapping),
                    priority: current.priority - 1,
                    ..Node::default()
                };

                // the current node now holds only the common prefix
                current.children = vec![child];
                current.indices = vec![current.prefix.as_bytes()[common_prefix]];
                current.prefix = from_utf8(&prefix[..common_prefix])?.into();
                current.wild_child = false;
            }

            // the route has a common prefix, search deeper
            if prefix.len() > common_prefix {
                prefix = &prefix[common_prefix..];

                let next = prefix[0];

                // `/` after param
                if current.node_type == NodeType::Param && next == b'/' && current.children.len() == 1 {
                    current = &mut current.children[0];
                    current.priority += 1;

                    continue 'walk;
                }

                // find a child that matches the next path byte
                for mut i in 0..current.indices.len() {
                    // found a match
                    if next == current.indices[i] {
                        i = current.update_child_priority(i);
                        current = &mut current.children[i];
                        continue 'walk;
                    }
                }

                // not a wildcard and there is no matching child node, create a new one
                if !matches!(next, b':' | b'*') && current.node_type != NodeType::CatchAll {
                    current.indices.push(next);
                    let mut child = current.add_child(Node::default());
                    child = current.update_child_priority(child);

                    // insert into the new node
                    let last = current.children[child].insert_child(prefix, &route, val)?;
                    last.param_remapping = param_remapping;
                    return Ok(());
                }

                // inserting a wildcard, and this node already has a wildcard child
                if current.wild_child {
                    // wildcards are always at the end
                    current = current.children.last_mut().unwrap();
                    current.priority += 1;

                    // make sure the wildcard matches
                    if prefix.len() < current.prefix.len()
                        || current.prefix.as_bytes() != &prefix[..current.prefix.len()]
                        // catch-alls cannot have children 
                        || current.node_type == NodeType::CatchAll
                        // check for longer wildcard, e.g. :name and :names
                        || (current.prefix.len() < prefix.len()
                            && prefix[current.prefix.len()] != b'/')
                    {
                        return Err(InsertError::conflict(&route, prefix, current));
                    }

                    continue 'walk;
                }

                // otherwise, create the wildcard node
                let last = current.insert_child(prefix, &route, val)?;
                last.param_remapping = param_remapping;
                return Ok(());
            }

            // exact match, this node should be empty
            if current.value.is_some() {
                return Err(InsertError::conflict(&route, prefix, current));
            }

            // add the value to current node
            current.value = Some(val);
            current.param_remapping = param_remapping;

            return Ok(());
        }
    }

    // add a child node, keeping wildcards at the end
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

    // increments priority of the given child and reorders if necessary.
    //
    // returns the new index of the child
    fn update_child_priority(&mut self, i: usize) -> usize {
        self.children[i].priority += 1;
        let priority = self.children[i].priority;

        // adjust position (move to front)
        let mut updated = i;
        while updated > 0 && self.children[updated - 1].priority < priority {
            // swap node positions
            self.children.swap(updated - 1, updated);
            updated -= 1;
        }

        // build new index list
        if updated != i {
            self.indices = [
                &self.indices[..updated],  // unchanged prefix, might be empty
                &self.indices[i..=i],      // the index char we move
                &self.indices[updated..i], // rest without char at 'pos'
                &self.indices[i + 1..],
            ]
            .concat();
        }

        updated
    }

    // insert a child node at this node
    fn insert_child(&mut self, mut prefix: &[u8], route: &[u8], val: T) -> Result<&mut Node<T>, InsertError> {
        let mut current = self;

        loop {
            // search for a wildcard segment
            let (wildcard, wildcard_index) = match find_wildcard(prefix)? {
                Some((w, i)) => (w, i),
                // no wildcard, simply use the current node
                None => {
                    current.value = Some(val);
                    current.prefix = from_utf8(prefix)?.into();
                    return Ok(current);
                }
            };

            // regular route parameter
            if wildcard[0] == b':' {
                // insert prefix before the current wildcard
                if wildcard_index > 0 {
                    current.prefix = from_utf8(&prefix[..wildcard_index])?.into();
                    prefix = &prefix[wildcard_index..];
                }

                let child = Self {
                    node_type: NodeType::Param,
                    prefix: from_utf8(wildcard)?.into(),
                    ..Self::default()
                };

                let child = current.add_child(child);
                current.wild_child = true;
                current = &mut current.children[child];
                current.priority += 1;

                // if the route doesn't end with the wildcard, then there
                // will be another non-wildcard subroute starting with '/'
                if wildcard.len() < prefix.len() {
                    prefix = &prefix[wildcard.len()..];
                    let child = Self {
                        priority: 1,
                        ..Self::default()
                    };

                    let child = current.add_child(child);
                    current = &mut current.children[child];
                    continue;
                }

                // otherwise we're done. Insert the value in the new leaf
                current.value = Some(val);
                return Ok(current);

            // catch-all route
            } else if wildcard[0] == b'*' {
                // "/foo/*x/bar"
                if wildcard_index + wildcard.len() != prefix.len() {
                    return Err(InsertError::InvalidCatchAll);
                }

                if let Some(i) = wildcard_index.checked_sub(1) {
                    // "/foo/bar*x"
                    if prefix[i] != b'/' {
                        return Err(InsertError::InvalidCatchAll);
                    }
                }

                // "*x" without leading `/`
                if prefix == route && route[0] != b'/' {
                    return Err(InsertError::InvalidCatchAll);
                }

                // insert prefix before the current wildcard
                if wildcard_index > 0 {
                    current.prefix = from_utf8(&prefix[..wildcard_index])?.into();
                    prefix = &prefix[wildcard_index..];
                }

                let child = Self {
                    prefix: from_utf8(prefix)?.into(),
                    node_type: NodeType::CatchAll,
                    value: Some(val),
                    priority: 1,
                    ..Self::default()
                };

                let i = current.add_child(child);
                current.wild_child = true;

                return Ok(&mut current.children[i]);
            }
        }
    }
}

struct Skipped<'n, 'p, T> {
    path: &'p str,
    node: &'n Node<T>,
    params: usize,
}

#[rustfmt::skip]
macro_rules! backtracker {
    ($skipped_nodes:ident, $path:ident, $current:ident, $params:ident, $backtracking:ident, $walk:lifetime) => {
        macro_rules! try_backtrack {
            () => {
                // try backtracking to any matching wildcard nodes we skipped while traversing
                // the tree
                while let Some(skipped) = $skipped_nodes.pop() {
                    if skipped.path.ends_with($path) {
                        $path = skipped.path;
                        $current = &skipped.node;
                        $params.truncate(skipped.params);
                        $backtracking = true;
                        continue $walk;
                    }
                }
            };
        }
    };
}

impl<T> Node<T> {
    // it's a bit sad that we have to introduce unsafe here but rust doesn't really have a way
    // to abstract over mutability, so `UnsafeCell` lets us avoid having to duplicate logic between
    // `at` and `at_mut`
    pub fn at(&self, full_path: &str) -> Result<(&T, Params), MatchError> {
        let mut current = self;
        let mut path = full_path;
        let mut backtracking = false;
        let mut params = Params::new();
        let mut skipped_nodes = Vec::new();

        'walk: loop {
            backtracker!(skipped_nodes, path, current, params, backtracking, 'walk);

            // the path is longer than this node's prefix, we are expecting a child node
            if path.len() > current.prefix.len() {
                let (prefix, rest) = path.split_at(current.prefix.len());

                // the prefix matches
                if prefix == current.prefix {
                    let consumed = path;
                    path = rest;

                    // try searching for a matching static child unless we are currently
                    // backtracking, which would mean we already traversed them
                    if !backtracking {
                        let first = path.as_bytes()[0];
                        if let Some(i) = current.indices.iter().position(|&c| c == first) {
                            // keep track of wildcard routes we skipped to backtrack to later if
                            // we don't find a math
                            if current.wild_child {
                                skipped_nodes.push(Skipped {
                                    path: consumed,
                                    node: current,
                                    params: params.len(),
                                });
                            }

                            // continue with the child node
                            current = &current.children[i];
                            continue 'walk;
                        }
                    }

                    // we didn't find a match and there are no children with wildcards, there is no match
                    if !current.wild_child {
                        try_backtrack!();

                        // nothing found
                        break;
                    }

                    // handle the wildcard child, which is always at the end of the list
                    current = current.children.last().unwrap();

                    match current.node_type {
                        NodeType::Param => {
                            // check if there are more segments in the path other than this parameter
                            match path.chars().position(|c| c == '/') {
                                Some(i) => {
                                    let (param, rest) = path.split_at(i);

                                    if let [child] = current.children.as_slice() {
                                        // store the parameter value
                                        params.push(&current.prefix[1..], param);

                                        // continue with the child node
                                        path = rest;
                                        current = child;
                                        backtracking = false;
                                        continue 'walk;
                                    }

                                    try_backtrack!();
                                }
                                // this is the last path segment
                                None => {
                                    // store the parameter value
                                    params.push(&current.prefix[1..], path);

                                    // found the matching value
                                    if let Some(ref value) = current.value {
                                        // remap parameter keys
                                        params
                                            .for_each_key_mut(|(i, key)| *key = current.param_remapping[i][1..].into());

                                        return Ok((value, params));
                                    }

                                    try_backtrack!();

                                    // this node doesn't have the value, no match
                                }
                            }
                        }
                        NodeType::CatchAll => {
                            // catch all segments are only allowed at the end of the route,
                            // either this node has the value or there is no match
                            if let Some(ref value) = current.value {
                                // remap parameter keys
                                params.for_each_key_mut(|(i, key)| *key = current.param_remapping[i][1..].into());

                                // store the final catch-all parameter
                                params.push(&current.prefix[1..], path);

                                return Ok((value, params));
                            }
                        }
                        _ => unreachable!(),
                    };

                    break;
                }
            }

            // this is it, we should have reached the node containing the value
            if current.prefix == path {
                if let Some(ref value) = current.value {
                    // remap parameter keys
                    params.for_each_key_mut(|(i, key)| *key = current.param_remapping[i][1..].into());
                    return Ok((value, params));
                }

                try_backtrack!();

                break;
            }

            try_backtrack!();

            break;
        }

        // a non aggressive way to handle single * wildcard
        if current.prefix == path {
            if let Some(val) = current.children.first() {
                if val.prefix == "*" {
                    if let Some(ref val) = val.value {
                        return Ok((val, params));
                    }
                }
            }
        }

        Err(MatchError)
    }

    #[cfg(feature = "__test_helpers")]
    pub fn check_priorities(&self) -> Result<u32, (u32, u32)> {
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

/// An ordered list of route parameters keys for a specific route, stored at leaf nodes.
type ParamRemapping = Vec<Box<str>>;

/// Returns `path` with normalized route parameters, and a parameter remapping
/// to store at the leaf node for this route.
fn normalize_params(mut path: Vec<u8>) -> Result<(Vec<u8>, ParamRemapping), InsertError> {
    let mut start = 0;
    let mut original = ParamRemapping::new();

    // parameter names are normalized alphabetically
    let mut next = b'a';

    loop {
        let (wildcard, mut wildcard_index) = match find_wildcard(&path[start..])? {
            Some((w, i)) => (w, i),
            None => return Ok((path, original)),
        };

        // makes sure the param has a valid name unless it's * catch all
        if wildcard.len() < 2 && wildcard.get(0).filter(|w| **w == b'*').is_none() {
            return Err(InsertError::UnnamedParam);
        }

        // don't need to normalize catch-all parameters
        if wildcard[0] == b'*' {
            start += wildcard_index + wildcard.len();
            continue;
        }

        wildcard_index += start;

        // normalize the parameter
        let removed = path.splice((wildcard_index)..(wildcard_index + wildcard.len()), vec![b':', next]);

        // remember the original name for remappings
        let removed = removed.collect::<Vec<u8>>();
        original.push(from_utf8(&removed[..])?.into());

        // get the next key
        next += 1;
        if next > b'z' {
            panic!("too many route parameters");
        }

        start = wildcard_index + 2;
    }
}

/// Restores `route` to it's original, denormalized form.
pub(crate) fn denormalize_params(route: &mut Vec<u8>, params: &ParamRemapping) {
    let mut start = 0;
    let mut i = 0;

    loop {
        // find the next wildcard
        let (wildcard, mut wildcard_index) = match find_wildcard(&route[start..]).unwrap() {
            Some((w, i)) => (w, i),
            None => return,
        };

        wildcard_index += start;

        let next = match params.get(i) {
            Some(param) => param.clone(),
            None => return,
        };

        // denormalize this parameter
        route.splice(
            (wildcard_index)..(wildcard_index + wildcard.len()),
            next.as_bytes().iter().copied(),
        );

        i += 1;
        start = wildcard_index + 2;
    }
}

// Searches for a wildcard segment and checks the path for invalid characters.
fn find_wildcard(path: &[u8]) -> Result<Option<(&[u8], usize)>, InsertError> {
    for (start, &c) in path.iter().enumerate() {
        // a wildcard starts with ':' (param) or '*' (catch-all)
        if c != b':' && c != b'*' {
            continue;
        }

        for (end, &c) in path[start + 1..].iter().enumerate() {
            match c {
                b'/' => return Ok(Some((&path[start..start + 1 + end], start))),
                b':' | b'*' => return Err(InsertError::TooManyParams),
                _ => {}
            }
        }

        return Ok(Some((&path[start..], start)));
    }

    Ok(None)
}

#[cfg(test)]
const _: () = {
    use std::fmt::{self, Debug, Formatter};

    // visualize the tree structure when debugging
    impl<T: Debug> Debug for Node<T> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            // safety: we only expose &mut T through &mut self
            let value = self.value.as_ref();

            let indices = self.indices.iter().map(|&x| char::from_u32(x as _)).collect::<Vec<_>>();

            let param_names = self.param_remapping.iter().map(|x| x.as_ref()).collect::<Vec<_>>();

            let mut fmt = f.debug_struct("Node");
            fmt.field("value", &value);
            fmt.field("prefix", &self.prefix);
            fmt.field("node_type", &self.node_type);
            fmt.field("children", &self.children);
            fmt.field("param_names", &param_names);
            fmt.field("indices", &indices);
            fmt.finish()
        }
    }
};
