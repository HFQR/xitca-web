use core::{pin::Pin, time::Duration};

use std::io;

use crate::date::DateTime;
use crate::util::timer::KeepAlive;

use crate::{date::DateTimeHandle, h2::dispatcher::Shared};

pub(crate) struct PingPong<'a> {
    timer: Pin<&'a mut KeepAlive>,
    ctx: &'a Shared,
    date: &'a DateTimeHandle,
    ka_dur: Duration,
}

impl<'a> PingPong<'a> {
    pub(crate) fn new(
        timer: Pin<&'a mut KeepAlive>,
        ctx: &'a Shared,
        date: &'a DateTimeHandle,
        ka_dur: Duration,
    ) -> Self {
        Self {
            timer,
            ctx,
            date,
            ka_dur,
        }
    }

    pub(crate) async fn tick(&mut self) -> io::Result<()> {
        self.timer.as_mut().await;

        self.ctx.borrow_mut().try_set_pending_ping()?;

        self.timer.as_mut().update(self.date.now() + self.ka_dur);

        Ok(())
    }
}
