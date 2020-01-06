use std::{
	collections::VecDeque,
	fmt::Debug,
	marker::Unpin,
	pin::Pin,
//	sync::Arc,
	time::{Duration, Instant},
};

//use codec::{Decode, Encode};

use futures::future::FutureExt;
use futures::prelude::Stream;
use futures::stream::{Fuse, StreamExt};
use futures::task::{Context, Poll};
use futures_timer::Delay;

pub type Interval = Box<dyn Stream<Item = ()> + Unpin + Send + Sync>;

pub fn interval_at(start: Instant, duration: Duration) -> Interval {
	let stream = futures::stream::unfold(start, move |next| {
		let time_until_next = next.saturating_duration_since(Instant::now());

		Delay::new(time_until_next).map(move |_| Some(((), next + duration)))
	});

	Box::new(stream)
}

pub struct PeriodicStream<S, M>
where
	S: Stream<Item = M>,
{
	incoming: Fuse<S>,
	check_pending: Interval,
	ready: VecDeque<M>,
}

impl<S, M> PeriodicStream<S, M>
where
	S: Stream<Item = M>,
{
	pub fn new(stream: S, duration: u64) -> Self {
		let dur = Duration::from_secs(duration);

		Self {
			incoming: stream.fuse(),
			check_pending: interval_at(Instant::now(), dur),
			ready: VecDeque::new(),
		}
	}
}

impl<S, M> Stream for PeriodicStream<S, M>
where
	S: Stream<Item = M> + Unpin,
	M: Debug + Unpin,
{
	type Item = S::Item;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
		loop {
			match self.incoming.poll_next_unpin(cx) {
				Poll::Ready(None) => break,
				Poll::Ready(Some(input)) => {
					self.ready.push_back(input);
				}
				Poll::Pending => break,
			}
		}

		let mut is_ready = false;
		if let Poll::Ready(Some(_)) = self.check_pending.poll_next_unpin(cx) {
			is_ready = true;
		}

		println!("ready? {:?}", is_ready);

		if let Some(ready) = self.ready.pop_front() {
			return Poll::Ready(Some(ready));
		}

		if !is_ready {
			return Poll::Pending;
		}

		if self.incoming.is_done() {
			Poll::Ready(None)
		} else {
			Poll::Pending
		}
	}
}

#[cfg(test)]
mod test {
	use futures::compat::{Compat, Stream01CompatExt};
	use futures::{
		future::{self, Future, FutureExt, TryFutureExt},
		stream,
	};

	use super::*;

	#[test]
	fn test_periodic() {
		struct F<S>
		where
			S: Stream<Item = u8>,
		{
			s: S,
		};

		impl<S> Future for F<S>
		where
			S: Stream<Item = u8> + Unpin,
		{
			type Output = ();

			fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
				loop {
					match self.s.poll_next_unpin(cx) {
						Poll::Ready(Some(item)) => {
							println!("item {:?}", item);
							assert_eq!(item, 1);
						}
						Poll::Ready(None) => {
							println!("finished");
							return Poll::Ready(());
						}
						Poll::Pending => {
							println!("pending");
							return Poll::Pending;
						}
					}
				}
			}
		}

		let s = stream::repeat(1u8).take(5);
		let f = F {
			s: PeriodicStream::<_, _>::new(s, 5),
		};

		futures::executor::block_on(f);
	}
}
