use std::{
    path::{self, Path},
    str::FromStr,
};

use crate::error::UriSegmentError;

#[derive(Debug)]
pub(crate) struct PathBuf(path::PathBuf);

impl FromStr for PathBuf {
    type Err = UriSegmentError;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        Self::parse_path(path, false)
    }
}

impl PathBuf {
    /// Parse a path, giving the choice of allowing hidden files to be considered valid segments.
    pub fn parse_path(path: &str, hidden_files: bool) -> Result<Self, UriSegmentError> {
        let mut buf = path::PathBuf::new();

        for segment in path.split('/') {
            if segment == ".." {
                buf.pop();
            } else if !hidden_files && segment.starts_with('.') {
                return Err(UriSegmentError::Start('.'));
            } else if segment.starts_with('*') {
                return Err(UriSegmentError::Start('*'));
            } else if segment.ends_with(':') {
                return Err(UriSegmentError::End(':'));
            } else if segment.ends_with('>') {
                return Err(UriSegmentError::End('>'));
            } else if segment.ends_with('<') {
                return Err(UriSegmentError::End('<'));
            } else if segment.is_empty() {
                continue;
            } else if cfg!(windows) && segment.contains('\\') {
                return Err(UriSegmentError::Char('\\'));
            } else {
                buf.push(segment)
            }
        }

        Ok(PathBuf(buf))
    }
}

impl AsRef<Path> for PathBuf {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::iter::FromIterator;

    use super::*;

    #[test]
    fn test_path_buf() {
        assert_eq!(
            PathBuf::from_str("/test/.tt").map(|t| t.0),
            Err(UriSegmentError::Start('.'))
        );
        assert_eq!(
            PathBuf::from_str("/test/*tt").map(|t| t.0),
            Err(UriSegmentError::Start('*'))
        );
        assert_eq!(
            PathBuf::from_str("/test/tt:").map(|t| t.0),
            Err(UriSegmentError::End(':'))
        );
        assert_eq!(
            PathBuf::from_str("/test/tt<").map(|t| t.0),
            Err(UriSegmentError::End('<'))
        );
        assert_eq!(
            PathBuf::from_str("/test/tt>").map(|t| t.0),
            Err(UriSegmentError::End('>'))
        );
        assert_eq!(
            PathBuf::from_str("/seg1/seg2/").unwrap().0,
            path::PathBuf::from_iter(vec!["seg1", "seg2"])
        );
        assert_eq!(
            PathBuf::from_str("/seg1/../seg2/").unwrap().0,
            path::PathBuf::from_iter(vec!["seg2"])
        );
    }

    #[test]
    fn test_parse_path() {
        assert_eq!(
            PathBuf::parse_path("/test/.tt", false).map(|t| t.0),
            Err(UriSegmentError::Start('.'))
        );

        assert_eq!(
            PathBuf::parse_path("/test/.tt", true).unwrap().0,
            path::PathBuf::from_iter(vec!["test", ".tt"])
        );
    }
}
