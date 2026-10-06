//! Value pages (spec §6.3): long strings, stored once and referenced from keys.
//!
//! Layout after the common header: a directory of `(offset u16, len u16)` per
//! record, then the record bytes packed from the end of the page down. The
//! header's `count` is the number of records. Value pages are always appended
//! and never rewritten or freed, and each checkpoint that writes long values
//! starts a fresh page, so a torn write can never damage a committed value
//! (§13).

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::PAGE_SIZE;
use crate::storage::StorageBackend;
use crate::storage::cache::PageCache;
use crate::storage::keys::{MAX_VALUE_BYTES, ValueRef};
use crate::storage::page::{PAGE_HEADER_SIZE, PAGE_TYPE_VALUE, PageAllocator, new_page};
use anyhow::Result;

const DIR_ENTRY: usize = 4;

/// Appends long values to fresh value pages for one checkpoint.
pub struct ValueWriter {
    /// The page being filled: its id and its records.
    current: Option<(u64, Vec<Vec<u8>>)>,
    /// Bytes the current page's records and directory use.
    used: usize,
}

impl Default for ValueWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ValueWriter {
    pub fn new() -> Self {
        ValueWriter {
            current: None,
            used: 0,
        }
    }

    /// Store `value` and return its ref. The page id is taken from `alloc` by
    /// appending when a page is started; a full page is written out then.
    pub fn push(
        &mut self,
        value: &[u8],
        alloc: &mut PageAllocator,
        backend: &mut dyn StorageBackend,
        cache: &PageCache,
    ) -> Result<ValueRef> {
        if value.len() > MAX_VALUE_BYTES {
            bail_coded!(
                ErrorCode::Int049,
                format!("value of {} bytes exceeds {MAX_VALUE_BYTES}", value.len())
            );
        }
        let need = value.len() + DIR_ENTRY;
        let fits = self.current.as_ref().is_some_and(|(_, records)| {
            records.len() < usize::from(u16::MAX)
                && PAGE_HEADER_SIZE + self.used + need <= PAGE_SIZE
        });
        if !fits {
            self.flush(alloc, backend, cache)?;
            self.current = Some((alloc.alloc_append()?, Vec::new()));
            self.used = 0;
        }
        let (page, records) = self
            .current
            .as_mut()
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "value page missing"))?;
        let slot = u16::try_from(records.len())
            .map_err(|_| err_coded!(ErrorCode::Int049, "too many values in a page"))?;
        records.push(value.to_vec());
        self.used += need;
        Ok(ValueRef { page: *page, slot })
    }

    /// Write the page being filled, if any.
    pub fn flush(
        &mut self,
        alloc: &mut PageAllocator,
        backend: &mut dyn StorageBackend,
        cache: &PageCache,
    ) -> Result<()> {
        let Some((id, records)) = self.current.take() else {
            return Ok(());
        };
        alloc.write(backend, cache, id, encode_value_page(&records)?)?;
        Ok(())
    }
}

/// Encode a value page holding `records` in slot order.
pub fn encode_value_page(records: &[Vec<u8>]) -> Result<Vec<u8>> {
    let count = u16::try_from(records.len())
        .map_err(|_| err_coded!(ErrorCode::Int049, "too many values in a page"))?;
    let used = PAGE_HEADER_SIZE + records.iter().map(|r| r.len() + DIR_ENTRY).sum::<usize>();
    if used > PAGE_SIZE {
        bail_coded!(ErrorCode::Int049, "value records overflow the page");
    }
    let mut page = new_page(PAGE_TYPE_VALUE, count);
    let mut end = PAGE_SIZE;
    for (i, record) in records.iter().enumerate() {
        let start = end - record.len();
        page.get_mut(start..end)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "value record past the page"))?
            .copy_from_slice(record);
        let dir = PAGE_HEADER_SIZE + i * DIR_ENTRY;
        let off = u16::try_from(start)
            .map_err(|_| err_coded!(ErrorCode::Int049, "value offset exceeds u16"))?;
        let len = u16::try_from(record.len())
            .map_err(|_| err_coded!(ErrorCode::Int049, "value length exceeds u16"))?;
        page.get_mut(dir..dir + 2)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "value directory past the page"))?
            .copy_from_slice(&off.to_le_bytes());
        page.get_mut(dir + 2..dir + 4)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "value directory past the page"))?
            .copy_from_slice(&len.to_le_bytes());
        end = start;
    }
    Ok(page)
}

/// The record in `slot` of a value page.
pub fn read_record(page: &[u8], page_id: u64, slot: u16) -> Result<Vec<u8>> {
    if page.first().copied() != Some(PAGE_TYPE_VALUE) {
        bail_coded!(ErrorCode::Stg013, page_id);
    }
    let count = crate::storage::page::page_count_field(page)?;
    if slot >= count {
        bail_coded!(
            ErrorCode::Int049,
            format!("value slot {slot} out of range on page {page_id}")
        );
    }
    let dir = PAGE_HEADER_SIZE + usize::from(slot) * DIR_ENTRY;
    let word = |at: usize| -> Result<usize> {
        page.get(at..at + 2)
            .and_then(|b| <[u8; 2]>::try_from(b).ok())
            .map(|b| usize::from(u16::from_le_bytes(b)))
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "value directory past the page"))
    };
    let (off, len) = (word(dir)?, word(dir + 2)?);
    let dir_end = PAGE_HEADER_SIZE + usize::from(count) * DIR_ENTRY;
    if off < dir_end {
        bail_coded!(ErrorCode::Int049, "value record overlaps the directory");
    }
    page.get(off..off + len)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "value record past the page"))
}

/// Read the long value at `vref` through the page cache.
pub fn read_value(
    vref: ValueRef,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<u8>> {
    let page = cache.get_or_load(vref.page, backend)?;
    read_record(&page[..], vref.page, vref.slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;

    #[test]
    fn values_round_trip_across_pages() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let mut alloc = PageAllocator::new(vec![2, 3], 10, 1);
        let mut writer = ValueWriter::new();
        let values: Vec<Vec<u8>> = (0..200u32)
            .map(|i| {
                vec![u8::try_from(i % 251).unwrap(); 65 + usize::try_from(i * 13 % 900).unwrap()]
            })
            .collect();
        let refs: Vec<ValueRef> = values
            .iter()
            .map(|v| writer.push(v, &mut alloc, &mut backend, &cache).unwrap())
            .collect();
        writer.flush(&mut alloc, &mut backend, &cache).unwrap();
        assert!(
            refs.iter().all(|r| r.page >= 10),
            "value pages always append"
        );
        assert!(refs.windows(2).all(|w| w[0] < w[1]), "refs increase");
        for (v, r) in values.iter().zip(&refs) {
            assert_eq!(&read_value(*r, &backend, &cache).unwrap(), v);
            crate::storage::page::verify(&backend.read_page(r.page).unwrap(), r.page, 1).unwrap();
        }
    }

    #[test]
    fn a_maximum_size_value_fills_one_page() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let mut alloc = PageAllocator::new(Vec::new(), 2, 1);
        let mut writer = ValueWriter::new();
        let max = vec![9u8; MAX_VALUE_BYTES];
        let a = writer.push(&max, &mut alloc, &mut backend, &cache).unwrap();
        let b = writer
            .push(b"next", &mut alloc, &mut backend, &cache)
            .unwrap();
        writer.flush(&mut alloc, &mut backend, &cache).unwrap();
        assert_ne!(a.page, b.page, "a full page is closed");
        assert_eq!(read_value(a, &backend, &cache).unwrap(), max);
        let too_big = vec![0u8; MAX_VALUE_BYTES + 1];
        assert!(
            writer
                .push(&too_big, &mut alloc, &mut backend, &cache)
                .is_err()
        );
    }

    #[test]
    fn bad_slot_or_page_type_is_an_error() {
        let page = encode_value_page(&[b"abc".to_vec()]).unwrap();
        assert_eq!(read_record(&page, 5, 0).unwrap(), b"abc");
        let err = read_record(&page, 5, 1).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-049");
        let leaf = crate::storage::node::encode_leaf(&[]).unwrap();
        let err = read_record(&leaf, 5, 0).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-013");
    }
}
