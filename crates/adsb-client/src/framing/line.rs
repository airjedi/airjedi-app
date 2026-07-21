use bytes::Bytes;

use super::{Frame, FrameType, Framer};

#[derive(Debug)]
pub struct LineFramer {
    buffer: Vec<u8>,
}

impl LineFramer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
        }
    }
}

impl Default for LineFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framer for LineFramer {
    fn feed(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    fn next_frame(&mut self) -> Option<Frame> {
        loop {
            let newline_pos = self.buffer.iter().position(|&b| b == b'\n')?;
            let line: Vec<u8> = self.buffer.drain(..=newline_pos).collect();
            let mut end = line.len();
            if end > 0 && line[end - 1] == b'\n' {
                end -= 1;
            }
            if end > 0 && line[end - 1] == b'\r' {
                end -= 1;
            }
            if end == 0 {
                continue;
            }
            return Some(Frame {
                timestamp: None,
                signal_level: None,
                data: Bytes::copy_from_slice(&line[..end]),
                frame_type: FrameType::TextLine,
            });
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line() {
        let mut framer = LineFramer::new();
        framer.feed(b"MSG,3,1,1,A1B2C3,1,2025/01/01,12:00:00.000\n");
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.frame_type, FrameType::TextLine);
        assert_eq!(frame.data.as_ref(), b"MSG,3,1,1,A1B2C3,1,2025/01/01,12:00:00.000");
        assert!(frame.timestamp.is_none());
        assert!(frame.signal_level.is_none());
    }

    #[test]
    fn crlf_line() {
        let mut framer = LineFramer::new();
        framer.feed(b"MSG,3,1,1,A1B2C3\r\n");
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.data.as_ref(), b"MSG,3,1,1,A1B2C3");
    }

    #[test]
    fn partial_line_returns_none() {
        let mut framer = LineFramer::new();
        framer.feed(b"MSG,3,1,1");
        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn partial_then_complete() {
        let mut framer = LineFramer::new();
        framer.feed(b"MSG,3,");
        assert!(framer.next_frame().is_none());

        framer.feed(b"1,1,A1B2C3\n");
        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), b"MSG,3,1,1,A1B2C3");
    }

    #[test]
    fn multiple_lines_in_one_feed() {
        let mut framer = LineFramer::new();
        framer.feed(b"line1\nline2\nline3\n");

        let f1 = framer.next_frame().unwrap();
        assert_eq!(f1.data.as_ref(), b"line1");

        let f2 = framer.next_frame().unwrap();
        assert_eq!(f2.data.as_ref(), b"line2");

        let f3 = framer.next_frame().unwrap();
        assert_eq!(f3.data.as_ref(), b"line3");

        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn empty_lines_skipped() {
        let mut framer = LineFramer::new();
        framer.feed(b"\n\ndata\n\n");

        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), b"data");
        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn crlf_empty_lines_skipped() {
        let mut framer = LineFramer::new();
        framer.feed(b"\r\n\r\ndata\r\n");

        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), b"data");
        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn reset_clears_buffer() {
        let mut framer = LineFramer::new();
        framer.feed(b"partial data without newline");
        framer.reset();
        assert!(framer.next_frame().is_none());

        framer.feed(b"fresh\n");
        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), b"fresh");
    }

    #[test]
    fn trailing_data_preserved() {
        let mut framer = LineFramer::new();
        framer.feed(b"line1\npartial");

        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), b"line1");
        assert!(framer.next_frame().is_none());

        framer.feed(b" complete\n");
        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), b"partial complete");
    }
}
