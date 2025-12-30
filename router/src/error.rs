use core::{error, fmt, ops::Deref};

use crate::{
    escape::{UnescapedRef, UnescapedRoute},
    tree::{denormalize_params, Node},
    String, Vec,
};

/// Represents errors that can occur when inserting a new route.
#[non_exhaustive]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InsertError {
    /// Attempted to insert a path that conflicts with an existing route.
    Conflict {
        /// The existing route that the insertion is conflicting with.
        with: String,
    },

    /// Only one parameter per route segment is allowed.
    ///
    /// For example, `/foo-{bar}` and `/{bar}-foo` are valid routes, but `/{foo}-{bar}`
    /// is not.
    InvalidParamSegment,

    /// Parameters must be registered with a valid name and matching braces.
    ///
    /// Note you can use `{{` or `}}` to escape literal brackets.
    InvalidParam,

    /// Catch-all parameters are only allowed at the end of a path.
    InvalidCatchAll,
}

impl fmt::Display for InsertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fmt = match self {
            Self::Conflict { with } => {
                return write!(
                    f,
                    "Insertion failed due to conflict with previously registered route: {with}"
                );
            }
            Self::InvalidParamSegment => "Only one parameter is allowed per path segment",
            Self::InvalidParam => "Parameters must be registered with a valid name",
            Self::InvalidCatchAll => "Catch-all parameters are only allowed at the end of a route",
        };
        f.write_str(fmt)
    }
}

impl error::Error for InsertError {}

impl InsertError {
    /// Returns an error for a route conflict with the given node.
    ///
    /// This method attempts to find the full conflicting route.
    pub(crate) fn conflict<T>(route: &UnescapedRoute, prefix: UnescapedRef<'_>, current: &Node<T>) -> Self {
        let mut route = route.clone();

        // The route is conflicting with the current node.
        if prefix.unescaped() == current.prefix.unescaped() {
            denormalize_params(&mut route, &current.remapping);
            return InsertError::Conflict {
                with: String::from_utf8(route.into_unescaped()).unwrap(),
            };
        }

        // Remove the non-matching suffix from the route.
        route.truncate(route.len() - prefix.len());

        // Add the conflicting prefix.
        if !route.ends_with(&current.prefix) {
            route.append(&current.prefix);
        }

        // Add the prefixes of the first conflicting child.
        let mut child = current.children.first();
        while let Some(node) = child {
            route.append(&node.prefix);
            child = node.children.first();
        }

        // Denormalize any route parameters.
        let mut last = current;
        while let Some(node) = last.children.first() {
            last = node;
        }
        denormalize_params(&mut route, &last.remapping);

        // Return the conflicting route.
        InsertError::Conflict {
            with: String::from_utf8(route.into_unescaped()).unwrap(),
        }
    }
}

/// A failed merge attempt.
///
/// See [`Router::merge`](crate::Router::merge) for details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeError(pub(crate) Vec<InsertError>);

impl MergeError {
    /// Returns a list of [`InsertError`] for every insertion that failed
    /// during the merge.
    pub fn into_errors(self) -> Vec<InsertError> {
        self.0
    }
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for error in self.0.iter() {
            writeln!(f, "{error}")?;
        }

        Ok(())
    }
}

impl error::Error for MergeError {}

impl Deref for MergeError {
    type Target = Vec<InsertError>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A failed match attempt.
///
/// ```
/// use matchit::{MatchError, Router};
/// # fn main() -> Result<(), Box<dyn core::error::Error>> {
/// let mut router = Router::new();
/// router.insert("/home", "Welcome!")?;
/// router.insert("/blog", "Our blog.")?;
///
/// // no routes match
/// if let Err(err) = router.at("/blo") {
///     assert_eq!(err, MatchError::NotFound);
/// }
/// # Ok(())
/// # }
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MatchError;

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Matching route not found")
    }
}

impl error::Error for MatchError {}
