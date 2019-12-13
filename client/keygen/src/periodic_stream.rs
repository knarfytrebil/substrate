use std::{
	collections::VecDeque,
	fmt::Debug,
	marker::Unpin,
	pin::Pin,
	//sync::Arc,
	time::{Duration, Instant},
};

//use codec::{Decode, Encode};

use futures03::compat::{ Stream01CompatExt};//Compat
use futures03::prelude::{Stream, };//TryStream
use futures03::stream::{ Fuse, StreamExt, };//FilterMap, TryStreamExt
use futures03::task::{Context, Poll};

// TODO change to tokio 0.2's interval when runtime changed to 0.2
use tokio_timer::Interval as Interval01;

pub struct PeriodicStream<S, M>
where
	S: Stream<Item = M>,
{
	incoming: Fuse<S>,
	check_pending: Pin<Box<dyn Stream<Item = Result<Instant, tokio_timer::Error>> + Send>>,
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
			check_pending: Interval01::new_interval(dur).compat().boxed(),
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
					let ready = &mut self.ready;
					ready.push_back(input);
				}
				Poll::Pending => break,
			}
		}

		if let Some(_) = match self.check_pending.poll_next_unpin(cx) {
			Poll::Ready(r) => match r.unwrap() {
				Ok(instant) => Some(instant),
				Err(e) => panic!(e),
			},
			Poll::Pending => return { Poll::Pending },
		} {}

		if let Some(ready) = self.ready.pop_front() {
			return Poll::Ready(Some(ready));
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

	use futures03::{
		future::{self, Future, FutureExt, TryFutureExt},
		stream,
	};
	use tokio::runtime::Runtime as Runtime01;

	use super::*;

	#[test]
	fn test_periodic() {
		let mut rt01 = Runtime01::new().unwrap();

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
			s: PeriodicStream::<_, u8>::new(s, 5),
		};

		let _ = rt01
			.block_on(f.map(|_| -> Result<(), ()> { Ok(()) }).compat())
			.unwrap();
	}
}
