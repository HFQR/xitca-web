use super::{tree::Node, InsertError, MatchError, Params};

/// A URL router.
///
/// See [the crate documentation](crate) for details.
#[derive(Clone)]
#[cfg_attr(test, derive(Debug))]
pub struct Router<T> {
    root: Node<T>,
}

impl<T> Router<T> {
    /// Construct a new router.
    pub const fn new() -> Self {
        Self { root: Node::new() }
    }

    /// Insert a route.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use xitca_router::Router;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut router = Router::new();
    /// router.insert("/home", "Welcome!")?;
    /// router.insert("/users/:id", "A User")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn insert(&mut self, route: impl Into<String>, value: T) -> Result<(), InsertError> {
        self.root.insert(route, value)
    }

    /// Tries to find a value in the router matching the given path.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use xitca_router::Router;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut router = Router::new();
    /// router.insert("/home", "Welcome!")?;
    ///
    /// let matched = router.at("/home").unwrap();
    /// assert_eq!(*matched.value, "Welcome!");
    /// # Ok(())
    /// # }
    /// ```
    pub fn at<'m, 'p>(&'m self, path: &'p str) -> Result<Match<'p, &'m T>, MatchError> {
        self.root
            .at(path.as_bytes())
            .map(|(value, params)| Match { value, params })
    }

    #[cfg(feature = "test_helpers")]
    pub fn check_priorities(&self) -> Result<u32, (u32, u32)> {
        self.root.check_priorities()
    }
}

/// A successful match consisting of the registered value
/// and URL parameters, returned by [`Router::at`](Router::at).
#[derive(Debug)]
pub struct Match<'v, V> {
    /// The value stored under the matched node.
    pub value: V,
    /// The route parameters. See [parameters](crate#parameters) for more details.
    pub params: Params<'v>,
}
