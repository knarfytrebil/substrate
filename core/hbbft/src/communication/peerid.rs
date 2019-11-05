use std::cmp::{Ord, Ordering, PartialOrd};
use std::fmt;

use derive_more::{From, Into};
use parity_codec::{Decode, Encode, Error as CodecError, Input};
use serde::de::Deserializer;
use serde::de::Error as SerdeError;
use serde::de::SeqAccess;
use serde::de::Visitor;
use serde::ser::SerializeSeq;
use serde::Serializer;
use serde::{Deserialize, Serialize};

use network::PeerId;

#[derive(Clone, Debug, Hash, PartialEq, Eq, From, Into)]
pub struct PeerIdW(pub PeerId);

impl PartialOrd for PeerIdW
{
  fn partial_cmp(&self, other: &Self) -> Option<Ordering>
  {
    Some(self.cmp(other))
  }
}

impl Ord for PeerIdW
{
  fn cmp(&self, other: &Self) -> Ordering
  {
    self.0.to_base58().cmp(&other.0.to_base58())
  }
}

impl Encode for PeerIdW
{
  fn encode(&self) -> Vec<u8>
  {
    let bytes = self.0.as_bytes();
    Encode::encode(bytes)
  }
}

impl Decode for PeerIdW
{
  fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError>
  {
    let decoded: Vec<u8> = Decode::decode(value)?;
    let pid = PeerId::from_bytes(decoded).unwrap();
    Ok(Self(pid))
  }
}

impl Serialize for PeerIdW
{
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let e = self.0.as_bytes();

    let mut seq = serializer.serialize_seq(Some(e.len()))?;
    for s in e
    {
      seq.serialize_element(s)?;
    }
    seq.end()
  }
}

struct PeerIdVisitor;

impl<'de> Visitor<'de> for PeerIdVisitor
{
  type Value = PeerIdW;

  fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result
  {
    formatter.write_str("sequence of bytes in PeerId shape")
  }

  fn visit_seq<M>(self, mut access: M) -> Result<Self::Value, M::Error>
  where
    M: SeqAccess<'de>,
  {
    let mut buf = Vec::<u8>::with_capacity(access.size_hint().unwrap_or(0));

    while let Some(value) = access.next_element()?
    {
      buf.push(value)
    }

    PeerId::from_bytes(buf)
      .map(|pid| PeerIdW(pid))
      .map_err(|_| M::Error::custom("PeerId from_bytes failed"))
  }
}

impl<'de> Deserialize<'de> for PeerIdW
{
  fn deserialize<D>(deserializer: D) -> Result<PeerIdW, D::Error>
  where
    D: Deserializer<'de>,
  {
    deserializer.deserialize_seq(PeerIdVisitor)
  }
}
