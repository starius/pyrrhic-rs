//! Bounds-checked access to table files. File offsets, rather than pointers,
//! identify every section while metadata is being parsed.

use std::{fs::File, io};

use memmap2::{Mmap, MmapOptions};

pub(crate) enum TableBytes {
    Mapped(Mmap),
    #[cfg(test)]
    Owned(Box<[u8]>),
}

impl TableBytes {
    pub(crate) fn map(file: &File) -> io::Result<Self> {
        // Syzygy tables are immutable inputs. The mapping must not be modified
        // or truncated while any generation still references it.
        let mapping = unsafe { MmapOptions::new().map(file)? };
        Ok(Self::Mapped(mapping))
    }

    #[cfg(test)]
    pub(crate) fn owned(bytes: impl Into<Box<[u8]>>) -> Self {
        Self::Owned(bytes.into())
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mapped(mapping) => mapping,
            #[cfg(test)]
            Self::Owned(bytes) => bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseError {
    Overflow,
    Truncated,
    InvalidAlignment,
}

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(crate) fn position(&self) -> usize {
        self.offset
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16, ParseError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("checked length");
        Ok(u16::from_le_bytes(bytes))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, ParseError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("checked length");
        Ok(u32::from_le_bytes(bytes))
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], ParseError> {
        let start = self.offset;
        let end = start.checked_add(len).ok_or(ParseError::Overflow)?;
        let section = self.bytes.get(start..end).ok_or(ParseError::Truncated)?;
        self.offset = end;
        Ok(section)
    }

    pub(crate) fn align(&mut self, alignment: usize) -> Result<(), ParseError> {
        if !alignment.is_power_of_two() {
            return Err(ParseError::InvalidAlignment);
        }
        let aligned = self
            .offset
            .checked_add(alignment - 1)
            .ok_or(ParseError::Overflow)?
            & !(alignment - 1);
        self.take(aligned - self.offset)?;
        Ok(())
    }

    pub(crate) fn section(&self, offset: usize, len: usize) -> Result<&'a [u8], ParseError> {
        let end = offset.checked_add(len).ok_or(ParseError::Overflow)?;
        self.bytes.get(offset..end).ok_or(ParseError::Truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, ParseError, TableBytes};

    #[test]
    fn checked_reads_keep_file_offsets_and_reject_invalid_ranges() {
        let storage = TableBytes::owned(vec![0xaa, 0x34, 0x12, 0, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]);
        let mut cursor = Cursor::new(storage.as_slice());
        assert_eq!(cursor.read_u8(), Ok(0xaa));
        assert_eq!(cursor.read_u16(), Ok(0x1234));
        cursor.align(8).unwrap();
        assert_eq!(cursor.position(), 8);
        assert_eq!(cursor.section(7, 4), Ok(&[0x78, 0x56, 0x34, 0x12][..]));
        assert_eq!(cursor.read_u32(), Err(ParseError::Truncated));
        assert_eq!(cursor.position(), 8);
        assert_eq!(cursor.section(1, usize::MAX), Err(ParseError::Overflow));
        assert_eq!(cursor.take(usize::MAX), Err(ParseError::Overflow));
        assert_eq!(cursor.align(3), Err(ParseError::InvalidAlignment));
    }
}
