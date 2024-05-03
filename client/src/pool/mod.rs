#![allow(dead_code)]

// pool for http/1 connections. connection is uniquely owned and ownership is exchanged between
// pool and caller.
pub(crate) mod exclusive;

// pool for http/2 and http/3 connections. connection is shared owned and ownership is reference
// counted between pool and caller.
pub(crate) mod shared;
