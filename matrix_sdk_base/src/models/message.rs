//! A queue that holds at most ten of the most recent messages.
//!
//! The `Room` struct optionally holds a `MessageQueue` if the "messages"
//! feature is enabled.

use std::cmp::Ordering;
use std::ops::{Deref, DerefMut};
use std::vec::IntoIter;

use crate::events::{AnyMessageEventContent, AnyMessageEventStub, MessageEventStub};

use serde::{de, ser, Serialize};

const MESSAGE_QUEUE_CAP: usize = 35;

pub type SyncMessageEvent = MessageEventStub<AnyMessageEventContent>;

/// A queue that holds the 35 most recent messages received from the server.
#[derive(Clone, Debug, Default)]
pub struct MessageQueue {
    pub(crate) msgs: Vec<MessageWrapper>,
}

/// A wrapper for `ruma_events::SyncMessageEvent` that allows implementation of
/// Eq, Ord and the Partial versions of the traits.
///
/// `MessageWrapper` also implements Deref and DerefMut so accessing the events contents
/// are simplified.
#[derive(Clone, Debug, Serialize)]
pub struct MessageWrapper(pub SyncMessageEvent);

impl MessageWrapper {
    pub fn clone_into_any_content(event: &AnyMessageEventStub) -> SyncMessageEvent {
        MessageEventStub {
            content: event.content(),
            sender: event.sender().clone(),
            origin_server_ts: *event.origin_server_ts(),
            event_id: event.event_id().clone(),
            unsigned: event.unsigned().clone(),
        }
    }
}

impl Deref for MessageWrapper {
    type Target = SyncMessageEvent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for MessageWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl PartialEq for MessageWrapper {
    fn eq(&self, other: &MessageWrapper) -> bool {
        self.0.event_id == other.0.event_id
    }
}

impl Eq for MessageWrapper {}

impl PartialOrd for MessageWrapper {
    fn partial_cmp(&self, other: &MessageWrapper) -> Option<Ordering> {
        Some(self.0.origin_server_ts.cmp(&other.0.origin_server_ts))
    }
}

impl Ord for MessageWrapper {
    fn cmp(&self, other: &MessageWrapper) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

impl PartialEq for MessageQueue {
    fn eq(&self, other: &MessageQueue) -> bool {
        self.msgs.len() == other.msgs.len()
            && self
                .msgs
                .iter()
                .zip(other.msgs.iter())
                .all(|(msg_a, msg_b)| msg_a.event_id == msg_b.event_id)
    }
}

impl MessageQueue {
    /// Create a new empty `MessageQueue`.
    pub fn new() -> Self {
        Self {
            msgs: Vec::with_capacity(45),
        }
    }

    /// Inserts a `MessageEvent` into `MessageQueue`, sorted by by `origin_server_ts`.
    ///
    /// Removes the oldest element in the queue if there are more than 10 elements.
    pub fn push(&mut self, msg: SyncMessageEvent) -> bool {
        // only push new messages into the queue
        if let Some(latest) = self.msgs.last() {
            if msg.origin_server_ts < latest.origin_server_ts && self.msgs.len() >= 10 {
                return false;
            }
        }

        let message = MessageWrapper(msg);
        match self.msgs.binary_search_by(|m| m.cmp(&message)) {
            Ok(pos) => {
                if self.msgs[pos] != message {
                    self.msgs.insert(pos, message)
                }
            }
            Err(pos) => self.msgs.insert(pos, message),
        }
        if self.msgs.len() > MESSAGE_QUEUE_CAP {
            self.msgs.remove(0);
        }
        true
    }

    pub fn iter(&self) -> impl Iterator<Item = &MessageWrapper> {
        self.msgs.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut MessageWrapper> {
        self.msgs.iter_mut()
    }
}

impl IntoIterator for MessageQueue {
    type Item = MessageWrapper;
    type IntoIter = IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.msgs.into_iter()
    }
}

pub(crate) mod ser_deser {
    use std::fmt;

    use super::*;

    struct MessageQueueDeserializer;

    impl<'de> de::Visitor<'de> for MessageQueueDeserializer {
        type Value = MessageQueue;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an array of message events")
        }

        fn visit_seq<S>(self, mut access: S) -> Result<Self::Value, S::Error>
        where
            S: de::SeqAccess<'de>,
        {
            let mut msgs = Vec::with_capacity(access.size_hint().unwrap_or(0));

            while let Some(msg) = access.next_element::<SyncMessageEvent>()? {
                msgs.push(MessageWrapper(msg));
            }

            Ok(MessageQueue { msgs })
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<MessageQueue, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_seq(MessageQueueDeserializer)
    }

    pub fn serialize<S>(msgs: &MessageQueue, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        msgs.msgs.serialize(serializer)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::collections::HashMap;
    use std::convert::TryFrom;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::*;

    use matrix_sdk_test::test_json;

    use crate::identifiers::{RoomId, UserId};
    use crate::Room;

    #[test]
    fn serialize() {
        let id = RoomId::try_from("!roomid:example.com").unwrap();
        let user = UserId::try_from("@example:example.com").unwrap();

        let mut room = Room::new(&id, &user);

        let json: &serde_json::Value = &test_json::MESSAGE_TEXT;
        let msg = serde_json::from_value::<SyncMessageEvent>(json.clone()).unwrap();

        let mut msgs = MessageQueue::new();
        msgs.push(msg.clone());
        room.messages = msgs;
        let mut joined_rooms = HashMap::new();
        joined_rooms.insert(id, room);

        assert_eq!(
            serde_json::json!({
                "!roomid:example.com": {
                    "room_id": "!roomid:example.com",
                    "room_name": {
                        "name": null,
                        "canonical_alias": null,
                        "aliases": [],
                        "heroes": [],
                        "joined_member_count": null,
                        "invited_member_count": null
                    },
                    "own_user_id": "@example:example.com",
                    "creator": null,
                    "joined_members": {},
                    "invited_members": {},
                    "messages": [ msg ],
                    "typing_users": [],
                    "power_levels": null,
                    "encrypted": null,
                    "unread_highlight": null,
                    "unread_notifications": null,
                    "tombstone": null
                }
            }),
            serde_json::to_value(&joined_rooms).unwrap()
        );
    }

    #[test]
    fn deserialize() {
        let id = RoomId::try_from("!roomid:example.com").unwrap();
        let user = UserId::try_from("@example:example.com").unwrap();

        let mut room = Room::new(&id, &user);

        let json: &serde_json::Value = &test_json::MESSAGE_TEXT;
        let msg = serde_json::from_value::<SyncMessageEvent>(json.clone()).unwrap();

        let mut msgs = MessageQueue::new();
        msgs.push(msg.clone());
        room.messages = msgs;

        let mut joined_rooms = HashMap::new();
        joined_rooms.insert(id, room);

        let json = serde_json::json!({
            "!roomid:example.com": {
                "room_id": "!roomid:example.com",
                "room_name": {
                    "name": null,
                    "canonical_alias": null,
                    "aliases": [],
                    "heroes": [],
                    "joined_member_count": null,
                    "invited_member_count": null
                },
                "own_user_id": "@example:example.com",
                "creator": null,
                "joined_members": {},
                "invited_members": {},
                "messages": [ msg ],
                "typing_users": [],
                "power_levels": null,
                "encrypted": null,
                "unread_highlight": null,
                "unread_notifications": null,
                "tombstone": null
            }
        });
        assert_eq!(
            joined_rooms,
            serde_json::from_value::<HashMap<RoomId, Room>>(json).unwrap()
        );
    }
}
