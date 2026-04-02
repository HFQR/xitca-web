//! IO traits and utilities.
//!
//! Completion-based async IO traits ([`AsyncBufRead`], [`AsyncBufWrite`]) are
//! always available. The remaining io_uring operation types require the
//! `runtime-uring` feature.

#[cfg(feature = "runtime-uring")]
mod accept;

#[cfg(feature = "runtime")]
mod async_buf_read_write_impl;

#[cfg(feature = "runtime-uring")]
mod close;

#[cfg(feature = "runtime-uring")]
mod connect;

#[cfg(feature = "runtime-uring")]
mod fallocate;

#[cfg(feature = "runtime-uring")]
mod fsync;

#[cfg(feature = "runtime-uring")]
mod mkdir_at;

#[cfg(feature = "runtime-uring")]
mod noop;
#[cfg(feature = "runtime-uring")]
pub(crate) use noop::NoOp;

#[cfg(feature = "runtime-uring")]
mod open;

#[cfg(feature = "runtime-uring")]
mod read;

#[cfg(feature = "runtime-uring")]
mod read_fixed;

#[cfg(feature = "runtime-uring")]
mod readv;

#[cfg(feature = "runtime-uring")]
mod recv_from;

#[cfg(feature = "runtime-uring")]
mod recvmsg;

#[cfg(feature = "runtime-uring")]
mod rename_at;

#[cfg(feature = "runtime-uring")]
mod send_to;

#[cfg(feature = "runtime-uring")]
mod send_zc;

#[cfg(feature = "runtime-uring")]
mod sendmsg;

#[cfg(feature = "runtime-uring")]
mod sendmsg_zc;

#[cfg(feature = "runtime-uring")]
mod shared_fd;
#[cfg(feature = "runtime-uring")]
pub(crate) use shared_fd::SharedFd;

#[cfg(feature = "runtime-uring")]
mod socket;
#[cfg(feature = "runtime-uring")]
pub(crate) use socket::Socket;

#[cfg(feature = "runtime-uring")]
mod statx;

#[cfg(feature = "runtime-uring")]
mod symlink;

#[cfg(feature = "runtime-uring")]
mod unlink_at;

#[cfg(feature = "runtime-uring")]
mod util;
#[cfg(feature = "runtime-uring")]
pub(crate) use util::cstr;

#[cfg(feature = "runtime-uring")]
pub(crate) mod write;

#[cfg(feature = "runtime-uring")]
mod write_fixed;

#[cfg(feature = "runtime-uring")]
mod writev;

#[cfg(feature = "runtime-uring")]
mod writev_all;
#[cfg(feature = "runtime-uring")]
pub(crate) use writev_all::writev_at_all;

mod async_buf_read;
mod async_buf_write;

pub use async_buf_read::AsyncBufRead;
pub use async_buf_write::{AsyncBufWrite, write_all};
