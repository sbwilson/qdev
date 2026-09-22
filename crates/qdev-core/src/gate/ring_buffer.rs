use std::collections::VecDeque;
use std::io::{self, Write};

pub const DEFAULT_HEAD_CAPACITY: usize = 64 * 1024; // 65,536 bytes
pub const DEFAULT_TAIL_CAPACITY: usize = 960 * 1024; // 983,040 bytes

/// Bounded Head+Tail ring buffer implementation.
/// Retains an immutable head buffer and a rolling tail buffer.
/// When stream exceeds head + tail capacity, incoming bytes displace the oldest
/// bytes in the tail buffer and increment `truncated_bytes`.
#[derive(Debug, Clone)]
pub struct HeadTailBuffer {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    truncated_bytes: usize,
}

impl HeadTailBuffer {
    /// Constructs a new HeadTailBuffer with default 64 KB head and 960 KB tail capacities.
    pub fn new() -> Self {
        Self::with_capacities(DEFAULT_HEAD_CAPACITY, DEFAULT_TAIL_CAPACITY)
    }

    /// Constructs a new HeadTailBuffer with customized head and tail capacities.
    pub fn with_capacities(head_cap: usize, tail_cap: usize) -> Self {
        Self {
            head: Vec::with_capacity(head_cap.min(4096)),
            tail: VecDeque::with_capacity(tail_cap.min(4096)),
            head_cap,
            tail_cap,
            truncated_bytes: 0,
        }
    }

    /// Writes raw bytes to the buffer.
    pub fn write_bytes(&mut self, mut buf: &[u8]) {
        if buf.is_empty() {
            return;
        }

        // 1. Fill head if space available
        if self.head.len() < self.head_cap {
            let take = (self.head_cap - self.head.len()).min(buf.len());
            self.head.extend_from_slice(&buf[..take]);
            buf = &buf[take..];
        }

        if buf.is_empty() {
            return;
        }

        // 2. Fill tail if space available
        if self.tail.len() < self.tail_cap {
            let take = (self.tail_cap - self.tail.len()).min(buf.len());
            self.tail.extend(&buf[..take]);
            buf = &buf[take..];
        }

        if buf.is_empty() {
            return;
        }

        // 3. Overflow handling in rolling tail
        if self.tail_cap == 0 {
            self.truncated_bytes += buf.len();
            return;
        }

        if buf.len() >= self.tail_cap {
            self.truncated_bytes += self.tail.len() + (buf.len() - self.tail_cap);
            self.tail.clear();
            self.tail.extend(&buf[buf.len() - self.tail_cap..]);
        } else {
            self.truncated_bytes += buf.len();
            self.tail.drain(0..buf.len());
            self.tail.extend(buf);
        }
    }

    /// Returns the number of bytes that have been truncated.
    pub fn truncated_bytes(&self) -> usize {
        self.truncated_bytes
    }

    /// Returns true if any bytes were truncated.
    pub fn is_truncated(&self) -> bool {
        self.truncated_bytes > 0
    }

    /// Returns the total number of retained bytes in head and tail.
    pub fn len(&self) -> usize {
        self.head.len() + self.tail.len()
    }

    /// Returns true if no bytes are retained in head or tail.
    pub fn is_empty(&self) -> bool {
        self.head.is_empty() && self.tail.is_empty()
    }

    /// Assembles and returns the entire buffer content as a `Vec<u8>`.
    /// If no truncation occurred, returns head + tail contiguously.
    /// If truncation occurred, returns head + `\n[... qdev: <N> bytes truncated ...]\n` + tail.
    pub fn to_bytes(&self) -> Vec<u8> {
        let marker = if self.truncated_bytes > 0 {
            Some(format!(
                "\n[... qdev: {} bytes truncated ...]\n",
                self.truncated_bytes
            ))
        } else {
            None
        };

        let marker_len = marker.as_ref().map(|m| m.len()).unwrap_or(0);
        let mut out = Vec::with_capacity(self.head.len() + marker_len + self.tail.len());
        out.extend_from_slice(&self.head);
        if let Some(m) = marker {
            out.extend_from_slice(m.as_bytes());
        }
        let (s1, s2) = self.tail.as_slices();
        out.extend_from_slice(s1);
        out.extend_from_slice(s2);
        out
    }

    /// Assembles and converts the entire buffer content to a UTF-8 lossy string.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.to_bytes()).into_owned()
    }
}

impl Write for HeadTailBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Default for HeadTailBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_under_capacity_no_marker() {
        let mut buf = HeadTailBuffer::with_capacities(5, 5);
        buf.write_bytes(b"hello");
        buf.write_bytes(b"world");
        assert_eq!(buf.truncated_bytes(), 0);
        assert!(!buf.is_truncated());
        assert_eq!(buf.to_bytes(), b"helloworld");
        assert_eq!(buf.to_string_lossy(), "helloworld");
    }

    #[test]
    fn test_buffer_overflow_with_marker() {
        let mut buf = HeadTailBuffer::with_capacities(4, 6);
        buf.write_bytes(b"0123456789"); // exactly 10 bytes: 4 in head, 6 in tail
        assert_eq!(buf.truncated_bytes(), 0);
        assert_eq!(buf.to_string_lossy(), "0123456789");

        // write 1 byte 'A'
        buf.write_bytes(b"A");
        assert_eq!(buf.truncated_bytes(), 1);
        assert!(buf.is_truncated());
        let expected = "0123\n[... qdev: 1 bytes truncated ...]\n56789A";
        assert_eq!(buf.to_string_lossy(), expected);

        // write 5 more bytes
        buf.write_bytes(b"BCDEF");
        assert_eq!(buf.truncated_bytes(), 6);
        let expected2 = "0123\n[... qdev: 6 bytes truncated ...]\nABCDEF";
        assert_eq!(buf.to_string_lossy(), expected2);
    }

    #[test]
    fn test_buffer_large_overflow_chunk() {
        let mut buf = HeadTailBuffer::with_capacities(4, 4);
        buf.write_bytes(b"01234567"); // 8 bytes
        assert_eq!(buf.truncated_bytes(), 0);

        // write 20 bytes at once
        buf.write_bytes(b"abcdefghijklmnopqrst");
        assert_eq!(buf.truncated_bytes(), 20);
        let output = buf.to_string_lossy();
        assert!(output.starts_with("0123\n[... qdev: 20 bytes truncated ...]\n"));
        assert!(output.ends_with("qrst"));
    }
}
