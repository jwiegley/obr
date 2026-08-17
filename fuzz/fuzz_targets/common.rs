#![allow(dead_code)]

use obr::model::{IssueType, Status};

pub(crate) struct ByteCursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> ByteCursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub(crate) fn next_byte(&mut self) -> u8 {
        if self.data.is_empty() {
            return 0;
        }
        let byte = self.data[self.offset % self.data.len()];
        self.offset = self.offset.wrapping_add(1);
        byte
    }

    pub(crate) fn next_u16(&mut self) -> u16 {
        u16::from(self.next_byte()) | (u16::from(self.next_byte()) << 8)
    }

    pub(crate) fn next_bool(&mut self) -> bool {
        self.next_byte() & 1 == 1
    }

    pub(crate) fn usize(&mut self, max_exclusive: usize) -> usize {
        if max_exclusive == 0 {
            0
        } else {
            usize::from(self.next_byte()) % max_exclusive
        }
    }

    pub(crate) fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.usize(max_len + 1);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(self.next_byte());
        }
        bytes
    }

    pub(crate) fn optional_text(&mut self, max_len: usize) -> Option<String> {
        if self.next_byte().is_multiple_of(4) {
            None
        } else {
            Some(self.text(max_len))
        }
    }

    pub(crate) fn text(&mut self, max_len: usize) -> String {
        String::from_utf8_lossy(&self.bytes(max_len)).into_owned()
    }
}

pub(crate) trait TrimmedCustomIssueCursorExt {
    fn prefix(&mut self) -> &'static str;
    fn status(&mut self) -> Status;
    fn issue_type(&mut self) -> IssueType;
}

impl TrimmedCustomIssueCursorExt for ByteCursor<'_> {
    fn prefix(&mut self) -> &'static str {
        match self.next_byte() % 4 {
            0 => "bd",
            1 => "br",
            2 => "sync",
            _ => "other",
        }
    }

    fn status(&mut self) -> Status {
        match self.next_byte() % 8 {
            0 => Status::Open,
            1 => Status::InProgress,
            2 => Status::Blocked,
            3 => Status::Deferred,
            4 => Status::Draft,
            5 => Status::Closed,
            6 => Status::Tombstone,
            _ => Status::Custom(non_blank(self.text(32), "custom-status")),
        }
    }

    fn issue_type(&mut self) -> IssueType {
        match self.next_byte() % 8 {
            0 => IssueType::Task,
            1 => IssueType::Bug,
            2 => IssueType::Feature,
            3 => IssueType::Epic,
            4 => IssueType::Chore,
            5 => IssueType::Docs,
            6 => IssueType::Question,
            _ => IssueType::Custom(non_blank(self.text(32), "custom-type")),
        }
    }
}

pub(crate) trait EmptyCustomIssueCursorExt {
    fn status(&mut self) -> Status;
    fn issue_type(&mut self) -> IssueType;
}

impl EmptyCustomIssueCursorExt for ByteCursor<'_> {
    fn status(&mut self) -> Status {
        match self.next_byte() % 8 {
            0 => Status::Open,
            1 => Status::InProgress,
            2 => Status::Blocked,
            3 => Status::Deferred,
            4 => Status::Draft,
            5 => Status::Closed,
            6 => Status::Tombstone,
            _ => Status::Custom(non_empty(self.text(32), "custom-status")),
        }
    }

    fn issue_type(&mut self) -> IssueType {
        match self.next_byte() % 8 {
            0 => IssueType::Task,
            1 => IssueType::Bug,
            2 => IssueType::Feature,
            3 => IssueType::Epic,
            4 => IssueType::Chore,
            5 => IssueType::Docs,
            6 => IssueType::Question,
            _ => IssueType::Custom(non_empty(self.text(32), "custom-type")),
        }
    }
}

pub(crate) trait BuiltInIssueCursorExt {
    fn status(&mut self) -> Status;
    fn issue_type(&mut self) -> IssueType;
}

impl BuiltInIssueCursorExt for ByteCursor<'_> {
    fn status(&mut self) -> Status {
        match self.next_byte() % 7 {
            0 => Status::Open,
            1 => Status::InProgress,
            2 => Status::Blocked,
            3 => Status::Deferred,
            4 => Status::Draft,
            5 => Status::Closed,
            _ => Status::Tombstone,
        }
    }

    fn issue_type(&mut self) -> IssueType {
        match self.next_byte() % 7 {
            0 => IssueType::Task,
            1 => IssueType::Bug,
            2 => IssueType::Feature,
            3 => IssueType::Epic,
            4 => IssueType::Chore,
            5 => IssueType::Docs,
            _ => IssueType::Question,
        }
    }
}

fn non_blank(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}
