// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Minimal ASN.1-like codec used by the XMM7360 RPC protocol.
//!
//! The firmware speaks a small, peculiar encoding: integers are `0x02 <len>
//! <big-endian bytes>` and strings are length-prefixed byte buffers tagged
//! `0x55` (8-bit elements), `0x56` (16-bit) or `0x57` (32-bit). Strings carry
//! both a logical element count and a padding count. This module reproduces
//! the encoder/decoder from the reverse-engineered `xmm7360-pci` implementation
//! byte-for-byte.

/// Tag for integers.
pub const TAG_INT: u8 = 0x02;
/// String tag for 8-bit elements.
pub const TAG_STRING_U8: u8 = 0x55;
/// String tag for 16-bit elements.
pub const TAG_STRING_U16: u8 = 0x56;
/// String tag for 32-bit elements.
pub const TAG_STRING_U32: u8 = 0x57;

/// Errors produced while encoding or decoding firmware payloads.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodecError {
    /// Ran out of input while decoding.
    #[error("unexpected end of buffer while parsing")]
    UnexpectedEof,
    /// An integer field did not start with the `0x02` tag.
    #[error("expected integer tag 0x02, got {0:#04x}")]
    BadIntTag(u8),
    /// An integer field declared an unsupported width.
    #[error("unsupported integer width {0}")]
    BadIntWidth(u8),
    /// A string field did not start with a known string tag.
    #[error("expected string tag 0x55/0x56/0x57, got {0:#04x}")]
    BadStringTag(u8),
    /// A value did not fit in its declared field.
    #[error("string of {valid} bytes exceeds capacity {capacity}")]
    StringOverflow {
        /// Number of bytes supplied.
        valid: usize,
        /// Declared field capacity.
        capacity: usize,
    },
    /// The format string contained an unknown specifier.
    #[error("unknown format character {0:?}")]
    BadFormat(char),
    /// The format string requested a length without digits.
    #[error("string specifier has no length")]
    MissingLength,
    /// The argument list was shorter than the format string.
    #[error("not enough arguments for format string")]
    MissingArgument,
    /// An argument had the wrong type for its format specifier.
    #[error("argument type does not match the format specifier")]
    WrongArgument,
    /// The argument list was longer than the format string.
    #[error("too many arguments supplied")]
    TooManyArguments,
    /// A decoded string did not match its declared element/padding count.
    #[error("string length mismatch: count {count} != valid {valid} + padding {padding}")]
    StringLengthMismatch {
        /// Declared total element count.
        count: usize,
        /// Declared valid element count.
        valid: usize,
        /// Declared padding element count.
        padding: usize,
    },
}

/// Encode a 32-bit integer as an ASN.1 integer field.
pub const fn asn_int(value: u32) -> [u8; 6] {
    let b = value.to_be_bytes();
    [TAG_INT, 4, b[0], b[1], b[2], b[3]]
}

/// Encode a single 32-bit integer (shorthand for `pack("L", &[Arg::Int(v)])`).
pub fn pack_u32(value: u32) -> Vec<u8> {
    asn_int(value).to_vec()
}

/// A typed argument accepted by [`pack`].
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    /// An integer (`B`, `H` or `L`).
    Int(u32),
    /// A byte string (`s` or `S`).
    Bytes(&'a [u8]),
}

/// A decoded firmware value.
#[derive(Clone, PartialEq, Eq)]
pub enum Value {
    /// An integer field.
    Int(u32),
    /// A string field, with padding stripped.
    Bytes(Vec<u8>),
}

impl Value {
    /// Borrow the value as an integer, if it is one.
    pub fn as_int(&self) -> Option<u32> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Bytes(_) => None,
        }
    }

    /// Borrow the value as bytes, if it is a string.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(v) => Some(v),
            Value::Int(_) => None,
        }
    }
}

impl core::fmt::Debug for Value {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Value::Int(v) => write!(f, "0x{v:x}"),
            Value::Bytes(v) => write!(f, "{v:?}"),
        }
    }
}

/// Encode `args` according to `fmt`.
///
/// The format language mirrors the original implementation:
///
/// * `B`/`H`/`L` — 8/16/32-bit integer,
/// * `s<len>` — byte string with capacity `<len>`,
/// * `S<elem><len>` — string with an explicit element type (`B`, `H` or `L`).
pub fn pack(fmt: &str, args: &[Arg<'_>]) -> Result<Vec<u8>, CodecError> {
    let mut fmt: Vec<char> = fmt.chars().collect();
    let mut args = args.iter();
    let mut out = Vec::new();

    while !fmt.is_empty() {
        let arg = args.next().ok_or(CodecError::MissingArgument)?;
        let ch = fmt.remove(0);
        match ch {
            'B' | 'H' | 'L' => {
                let Arg::Int(value) = arg else {
                    return Err(CodecError::WrongArgument);
                };
                let width = match ch {
                    'B' => 1,
                    'H' => 2,
                    _ => 4,
                };
                out.push(TAG_INT);
                out.push(width);
                let bytes = value.to_be_bytes();
                out.extend_from_slice(&bytes[4 - width as usize..]);
            }
            's' => {
                let Arg::Bytes(value) = arg else {
                    return Err(CodecError::WrongArgument);
                };
                pack_string(value, 1, &mut fmt, &mut out)?;
            }
            'S' => {
                let elem = fmt.first().copied().ok_or(CodecError::MissingLength)?;
                fmt.remove(0);
                let (elem_size, arg) = match elem {
                    'B' => (1, arg),
                    'H' => (2, arg),
                    'L' => (4, arg),
                    other => return Err(CodecError::BadFormat(other)),
                };
                let Arg::Bytes(value) = arg else {
                    return Err(CodecError::WrongArgument);
                };
                pack_string(value, elem_size, &mut fmt, &mut out)?;
            }
            other => return Err(CodecError::BadFormat(other)),
        }
    }

    if args.next().is_some() {
        return Err(CodecError::TooManyArguments);
    }
    Ok(out)
}

fn pack_string(
    value: &[u8],
    elem_size: usize,
    fmt: &mut Vec<char>,
    out: &mut Vec<u8>,
) -> Result<(), CodecError> {
    let mut length_str = String::new();
    while fmt.first().is_some_and(char::is_ascii_digit) {
        length_str.push(fmt.remove(0));
    }
    if length_str.is_empty() {
        return Err(CodecError::MissingLength);
    }
    // `length_str` is known to be ASCII digits.
    let length: usize = length_str.parse().map_err(|_| CodecError::MissingLength)?;

    if !value.len().is_multiple_of(elem_size) {
        return Err(CodecError::StringOverflow {
            valid: value.len(),
            capacity: value.len(),
        });
    }
    let elements = value.len() / elem_size;
    if elements > length {
        return Err(CodecError::StringOverflow {
            valid: elements,
            capacity: length,
        });
    }

    let field_tag = match elem_size {
        1 => TAG_STRING_U8,
        2 => TAG_STRING_U16,
        _ => TAG_STRING_U32,
    };
    let count = length * elem_size;
    let padding = count - value.len();

    out.push(field_tag);
    if elements < 128 {
        out.push(elements as u8);
    } else {
        // Big-endian variable length; the high bit marks continuation.
        let mut remain = elements;
        let mut bytes = Vec::new();
        while remain > 0 {
            bytes.push((remain & 0xff) as u8);
            remain >>= 8;
        }
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes.iter().rev());
    }
    out.extend_from_slice(&asn_int(count as u32));
    out.extend_from_slice(&asn_int(padding as u32));
    out.extend_from_slice(value);
    out.resize(out.len() + padding, 0);
    Ok(())
}

/// A cursor over a firmware payload.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Wrap a byte slice.
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Whether all input has been consumed.
    pub const fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Peek at the next byte without consuming it.
    pub fn peek_u8(&self) -> Result<u8, CodecError> {
        self.data
            .get(self.pos)
            .copied()
            .ok_or(CodecError::UnexpectedEof)
    }

    /// Consume the next byte.
    pub fn take_u8(&mut self) -> Result<u8, CodecError> {
        let byte = self.peek_u8()?;
        self.pos += 1;
        Ok(byte)
    }

    /// Consume the next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or(CodecError::UnexpectedEof)?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(CodecError::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    /// Consume an ASN.1 integer field.
    pub fn take_asn_int(&mut self) -> Result<u32, CodecError> {
        let tag = self.take_u8()?;
        if tag != TAG_INT {
            return Err(CodecError::BadIntTag(tag));
        }
        let width = self.take_u8()?;
        if width == 0 || width > 4 {
            return Err(CodecError::BadIntWidth(width));
        }
        let mut value = 0u32;
        for _ in 0..width {
            value = (value << 8) | u32::from(self.take_u8()?);
        }
        Ok(value)
    }

    /// Consume a string field, stripping padding.
    pub fn take_string(&mut self) -> Result<Vec<u8>, CodecError> {
        let tag = self.take_u8()?;
        let elem_size = match tag {
            TAG_STRING_U8 => 1,
            TAG_STRING_U16 => 2,
            TAG_STRING_U32 => 4,
            other => return Err(CodecError::BadStringTag(other)),
        };

        let mut valid = usize::from(self.take_u8()?);
        if valid & 0x80 != 0 {
            let n = valid & 0x0f;
            let mut value = 0usize;
            for i in 0..n {
                value |= usize::from(self.take_u8()?) << (i * 8);
            }
            valid = value;
        }
        valid *= elem_size;

        let count = self.take_asn_int()? as usize;
        let padding = self.take_asn_int()? as usize;
        if count != 0 && count != valid + padding {
            return Err(CodecError::StringLengthMismatch {
                count,
                valid,
                padding,
            });
        }

        let payload = self.take(valid)?.to_vec();
        self.take(padding)?;
        Ok(payload)
    }
}

/// Decode a sequence of values until the input is exhausted.
pub fn decode_values(data: &[u8]) -> Result<Vec<Value>, CodecError> {
    let mut reader = Reader::new(data);
    let mut out = Vec::new();
    while !reader.is_empty() {
        match reader.peek_u8()? {
            TAG_INT => out.push(Value::Int(reader.take_asn_int()?)),
            TAG_STRING_U8 | TAG_STRING_U16 | TAG_STRING_U32 => {
                out.push(Value::Bytes(reader.take_string()?));
            }
            other => return Err(CodecError::BadStringTag(other)),
        }
    }
    Ok(out)
}

/// Decode values according to a `fmt` string of `n` (integer) and `s` (string).
pub fn unpack(fmt: &str, data: &[u8]) -> Result<Vec<Value>, CodecError> {
    let mut reader = Reader::new(data);
    let mut out = Vec::new();
    for ch in fmt.chars() {
        match ch {
            'n' => out.push(Value::Int(reader.take_asn_int()?)),
            's' => out.push(Value::Bytes(reader.take_string()?)),
            other => return Err(CodecError::BadFormat(other)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_encoding_roundtrips() {
        for value in [0u32, 1, 0x0102_0304, u32::MAX] {
            let encoded = asn_int(value);
            let reader = &mut Reader::new(&encoded);
            assert_eq!(reader.take_asn_int().unwrap(), value);
        }
    }

    #[test]
    fn short_string_pads_to_capacity() {
        let out = pack("s4", &[Arg::Bytes(b"ab")]).unwrap();
        assert_eq!(
            out,
            [
                0x55, 0x02, 0x02, 0x04, 0, 0, 0, 4, 0x02, 0x04, 0, 0, 0, 2, b'a', b'b', 0, 0,
            ]
        );
        let decoded = unpack("s", &out).unwrap();
        assert_eq!(decoded[0].as_bytes().unwrap(), b"ab");
    }

    #[test]
    fn long_string_uses_variable_length() {
        // 200 bytes fits in a single extended length byte.
        let value = [0xAAu8; 200];
        let out = pack("s200", &[Arg::Bytes(&value)]).unwrap();
        assert_eq!(out[0], 0x55);
        assert_eq!(out[1], 0x81);
        assert_eq!(out[2], 200);
        let decoded = unpack("s", &out).unwrap();
        assert_eq!(decoded[0].as_bytes().unwrap(), &value);
    }

    #[test]
    fn rejects_overflow() {
        assert!(matches!(
            pack("s2", &[Arg::Bytes(b"abc")]),
            Err(CodecError::StringOverflow { .. })
        ));
    }
}
