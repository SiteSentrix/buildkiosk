//! Copy-on-write tables of records.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use buildkiosk_core::Id;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::ModelError;

/// A record stored in a [`Table`].
pub trait Record: Clone + fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The item kind in references, links and URLs (`"rfi"`). Stable once released.
    const KIND: &'static str;

    fn id(&self) -> Id<Self>;
}

/// The rows of one table, keyed by id, plus the next id to hand out.
///
/// Cloning is O(1): the rows are shared until one side changes, and then only the changed table is
/// copied (its row pointers, not the records). The engine runs every command on a clone of the
/// company state and keeps it only if the command succeeds, so a failed or panicking command
/// leaves nothing behind.
pub struct Table<T> {
    rows: Arc<BTreeMap<u64, Arc<T>>>,
    next: u64,
}

impl<T> Clone for Table<T> {
    fn clone(&self) -> Self {
        Table { rows: Arc::clone(&self.rows), next: self.next }
    }
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Table { rows: Arc::new(BTreeMap::new()), next: 1 }
    }
}

impl<T: fmt::Debug> fmt::Debug for Table<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table").field("next", &self.next).field("rows", &self.rows.len()).finish()
    }
}

impl<T: PartialEq> PartialEq for Table<T> {
    fn eq(&self, other: &Self) -> bool {
        self.next == other.next && (Arc::ptr_eq(&self.rows, &other.rows) || self.rows == other.rows)
    }
}

/// One table's part of a journal entry: the rows a command added, changed or removed, and the new
/// next id when it moved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableChange {
    #[serde(rename = "t")]
    pub table: String,
    #[serde(rename = "n", default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RowChange>,
}

/// A row as it is after the change, or `None` if it was removed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RowChange {
    pub id: u64,
    #[serde(rename = "v")]
    pub value: Option<serde_json::Value>,
}

/// A table as stored in a snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
pub struct TableData<T> {
    pub next: u64,
    pub rows: Vec<T>,
}

impl<T> Default for TableData<T> {
    fn default() -> Self {
        TableData { next: 1, rows: Vec::new() }
    }
}

impl<T: Record> Table<T> {
    pub fn get(&self, id: Id<T>) -> Option<&T> {
        self.rows.get(&id.raw()).map(Arc::as_ref)
    }

    pub fn contains(&self, id: Id<T>) -> bool {
        self.rows.contains_key(&id.raw())
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows in id order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + '_ {
        self.rows.values().map(Arc::as_ref)
    }

    /// The id the next [`Table::alloc`] returns.
    pub fn peek_next(&self) -> Id<T> {
        Id::new(self.next)
    }

    /// Hands out a fresh id. Ids are never reused, even after a row is removed.
    pub fn alloc(&mut self) -> Id<T> {
        let id = Id::new(self.next);
        self.next = self.next.saturating_add(1);
        id
    }

    /// Inserts or replaces the row with `rec.id()`.
    pub fn put(&mut self, rec: T) {
        let id = rec.id().raw();
        if id >= self.next {
            self.next = id.saturating_add(1);
        }
        Arc::make_mut(&mut self.rows).insert(id, Arc::new(rec));
    }

    /// Changes one row in place; `None` if there is no such row.
    pub fn update<R>(&mut self, id: Id<T>, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        if !self.rows.contains_key(&id.raw()) {
            return None;
        }
        let row = Arc::make_mut(&mut self.rows).get_mut(&id.raw())?;
        Some(f(Arc::make_mut(row)))
    }

    /// Removes a row for good (most tools move rows to the recycle bin instead).
    pub fn remove(&mut self, id: Id<T>) -> Option<T> {
        if !self.rows.contains_key(&id.raw()) {
            return None;
        }
        Arc::make_mut(&mut self.rows).remove(&id.raw()).map(|r| Arc::unwrap_or_clone(r))
    }

    /// What changed from `self` to `newer`, or `None` if nothing did.
    pub(crate) fn diff(&self, name: &str, newer: &Self) -> Result<Option<TableChange>, ModelError> {
        let mut rows = Vec::new();
        if !Arc::ptr_eq(&self.rows, &newer.rows) {
            let mut a = self.rows.iter().peekable();
            let mut b = newer.rows.iter().peekable();
            loop {
                match (a.peek(), b.peek()) {
                    (None, None) => break,
                    (Some((ka, _)), None) => {
                        rows.push(RowChange { id: **ka, value: None });
                        a.next();
                    }
                    (None, Some((kb, vb))) => {
                        rows.push(RowChange { id: **kb, value: Some(to_value(name, vb)?) });
                        b.next();
                    }
                    (Some((ka, va)), Some((kb, vb))) => match ka.cmp(kb) {
                        std::cmp::Ordering::Less => {
                            rows.push(RowChange { id: **ka, value: None });
                            a.next();
                        }
                        std::cmp::Ordering::Greater => {
                            rows.push(RowChange { id: **kb, value: Some(to_value(name, vb)?) });
                            b.next();
                        }
                        std::cmp::Ordering::Equal => {
                            if !Arc::ptr_eq(va, vb) && va != vb {
                                rows.push(RowChange { id: **kb, value: Some(to_value(name, vb)?) });
                            }
                            a.next();
                            b.next();
                        }
                    },
                }
            }
        }
        let next = (self.next != newer.next).then_some(newer.next);
        if rows.is_empty() && next.is_none() {
            return Ok(None);
        }
        Ok(Some(TableChange { table: name.to_string(), next, rows }))
    }

    /// Applies a change recorded by [`Table::diff`] (journal replay).
    pub(crate) fn apply(&mut self, change: &TableChange) -> Result<(), ModelError> {
        for row in &change.rows {
            match &row.value {
                None => {
                    Arc::make_mut(&mut self.rows).remove(&row.id);
                }
                Some(v) => {
                    let rec: T = serde_json::from_value(v.clone())
                        .map_err(|e| ModelError::Decode { table: change.table.clone(), msg: format!("row {}: {e}", row.id) })?;
                    if rec.id().raw() != row.id {
                        return Err(ModelError::Decode { table: change.table.clone(), msg: format!("row {} holds id {}", row.id, rec.id()) });
                    }
                    self.put(rec);
                }
            }
        }
        if let Some(n) = change.next {
            self.next = n.max(1);
        }
        Ok(())
    }

    pub(crate) fn to_data(&self) -> TableData<&T> {
        TableData { next: self.next, rows: self.rows.values().map(Arc::as_ref).collect() }
    }

    pub(crate) fn from_data(name: &str, data: TableData<T>) -> Result<Self, ModelError> {
        let mut t = Table::default();
        for rec in data.rows {
            if t.contains(rec.id()) {
                return Err(ModelError::Decode { table: name.to_string(), msg: format!("duplicate id {}", rec.id()) });
            }
            t.put(rec);
        }
        t.next = t.next.max(data.next);
        Ok(t)
    }
}

fn to_value<T: Serialize>(table: &str, rec: &T) -> Result<serde_json::Value, ModelError> {
    serde_json::to_value(rec).map_err(|e| ModelError::Encode { table: table.to_string(), msg: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Row {
        id: Id<Row>,
        name: String,
    }

    impl Record for Row {
        const KIND: &'static str = "row";
        fn id(&self) -> Id<Row> {
            self.id
        }
    }

    fn row(id: u64, name: &str) -> Row {
        Row { id: Id::new(id), name: name.into() }
    }

    #[test]
    fn copy_on_write_keeps_the_original() {
        let mut a: Table<Row> = Table::default();
        a.put(row(1, "one"));
        let mut b = a.clone();
        b.update(Id::new(1), |r| r.name = "uno".into());
        b.put(row(2, "two"));
        assert_eq!(a.get(Id::new(1)).unwrap().name, "one");
        assert_eq!(a.len(), 1);
        assert_eq!(b.get(Id::new(1)).unwrap().name, "uno");
        assert_eq!(b.peek_next(), Id::new(3));
    }

    #[test]
    fn diff_then_apply_reproduces_the_change() {
        let mut a: Table<Row> = Table::default();
        a.put(row(1, "one"));
        a.put(row(2, "two"));
        a.put(row(3, "three"));
        let mut b = a.clone();
        b.update(Id::new(1), |r| r.name = "uno".into());
        b.update(Id::new(2), |r| r.name = "two".into()); // same value: not a change
        b.remove(Id::new(3));
        let id = b.alloc();
        b.put(row(id.raw(), "four"));
        let _unused = b.alloc();
        let ch = a.diff("rows", &b).unwrap().unwrap();
        assert_eq!(ch.rows.iter().map(|r| (r.id, r.value.is_some())).collect::<Vec<_>>(), vec![(1, true), (3, false), (4, true)]);
        assert_eq!(ch.next, Some(6));
        let mut c = a.clone();
        c.apply(&ch).unwrap();
        assert_eq!(c, b);
        assert!(b.diff("rows", &b.clone()).unwrap().is_none());
    }

    #[test]
    fn apply_rejects_mismatched_rows() {
        let mut t: Table<Row> = Table::default();
        let ch =
            TableChange { table: "rows".into(), next: None, rows: vec![RowChange { id: 2, value: Some(serde_json::json!({"id": 3, "name": "x"})) }] };
        assert!(t.apply(&ch).is_err());
        let bad = TableChange { table: "rows".into(), next: None, rows: vec![RowChange { id: 2, value: Some(serde_json::json!({"id": "x"})) }] };
        assert!(t.apply(&bad).is_err());
    }
}
