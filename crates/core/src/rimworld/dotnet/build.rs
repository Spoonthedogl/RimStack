use super::{ASSEMBLY_REF, Col, METHOD_DEF, TABLES};

/// Start of the code in memory: after the .NET header that `finish` writes.
const CODE_RVA: u32 = 0x1048;

#[derive(Default)]
pub struct Builder {
    strings: Vec<u8>,
    blobs: Vec<u8>,
    user_strings: Vec<u8>,
    pub code: Vec<u8>,
    rows: Vec<Vec<Vec<u32>>>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            strings: vec![0],
            blobs: vec![0],
            user_strings: vec![0],
            code: Vec::new(),
            rows: vec![Vec::new(); TABLES.len()],
        }
    }

    pub fn string(&mut self, text: &str) -> u32 {
        let at = self.strings.len() as u32;
        self.strings.extend(text.as_bytes());
        self.strings.push(0);
        at
    }

    pub fn blob(&mut self, bytes: &[u8]) -> u32 {
        let at = self.blobs.len() as u32;
        self.blobs.push(bytes.len() as u8);
        self.blobs.extend(bytes);
        at
    }

    pub fn user_string(&mut self, text: &str) -> u32 {
        let at = self.user_strings.len() as u32;
        let units: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        self.user_strings.push(units.len() as u8 + 1);
        self.user_strings.extend(units);
        self.user_strings.push(0);
        at
    }

    pub fn member_ref(&mut self, type_ref: u32, name: &str) -> u32 {
        let (name, signature) = (self.string(name), self.blob(&[0x00, 0, 0x01]));
        self.row(super::MEMBER_REF, &[type_ref << 3 | 1, name, signature])
    }

    /// Adds a row and returns its number, from 1.
    pub fn row(&mut self, table: usize, cells: &[u32]) -> u32 {
        assert_eq!(cells.len(), TABLES[table].len());
        self.rows[table].push(cells.to_vec());
        self.rows[table].len() as u32
    }

    pub fn type_ref(&mut self, namespace: &str, name: &str) -> u32 {
        let (name, namespace) = (self.string(name), self.string(namespace));
        self.row(super::TYPE_REF, &[0, name, namespace])
    }

    /// `extends_ref` is a `TypeRef` row, 0 for none. The type's methods are
    /// the `MethodDef` rows added after it, until the next type.
    pub fn type_def(&mut self, namespace: &str, name: &str, extends_ref: u32) -> u32 {
        let (name, namespace) = (self.string(name), self.string(namespace));
        let extends = if extends_ref == 0 {
            0
        } else {
            extends_ref << 2 | 1
        };
        let next_method = self.rows[super::METHOD_DEF].len() as u32 + 1;
        let next_field = self.rows[super::FIELD].len() as u32 + 1;
        self.row(
            super::TYPE_DEF,
            &[0, name, namespace, extends, next_field, next_method],
        )
    }

    pub fn method(&mut self, name: &str, signature: &[u8]) -> u32 {
        let (name, signature) = (self.string(name), self.blob(signature));
        self.row(METHOD_DEF, &[0, 0, 0, name, signature, 1])
    }

    /// Tiny method header: the code is under 64 bytes.
    pub fn method_with(&mut self, name: &str, flags: u32, code: &[u8]) -> u32 {
        let (name, signature) = (self.string(name), self.blob(&[0x00, 0, 0x01]));
        let rva = CODE_RVA + self.code.len() as u32;
        self.code.push((code.len() as u8) << 2 | 2);
        self.code.extend(code);
        self.row(METHOD_DEF, &[rva, 0, flags, name, signature, 1])
    }

    pub fn field(&mut self, name: &str) -> u32 {
        let (name, signature) = (self.string(name), self.blob(&[0x06, 0x1c]));
        self.row(super::FIELD, &[0, name, signature])
    }

    /// A field of the last type added; `flags` 0x10 makes it static.
    pub fn field_of(&mut self, name: &str, flags: u32, signature: &[u8]) -> u32 {
        let (name, signature) = (self.string(name), self.blob(signature));
        self.row(super::FIELD, &[flags, name, signature])
    }

    pub fn implements(&mut self, class: u32, interface: u32) {
        self.row(super::INTERFACE_IMPL, &[class, interface << 2]);
    }

    pub fn nested(&mut self, inner: u32, outer: u32) {
        self.row(super::NESTED_CLASS, &[inner, outer]);
    }

    /// `extends_def` is a row of this assembly; 0 for none.
    pub fn type_def_under(&mut self, namespace: &str, name: &str, extends_def: u32) -> u32 {
        let (name, namespace) = (self.string(name), self.string(namespace));
        let next_method = self.rows[METHOD_DEF].len() as u32 + 1;
        let next_field = self.rows[super::FIELD].len() as u32 + 1;
        self.row(
            super::TYPE_DEF,
            &[
                0,
                name,
                namespace,
                extends_def << 2,
                next_field,
                next_method,
            ],
        )
    }

    pub fn constructor(&mut self, type_ref: u32, signature: &[u8]) -> u32 {
        let (name, signature) = (self.string(".ctor"), self.blob(signature));
        self.row(super::MEMBER_REF, &[type_ref << 3 | 1, name, signature])
    }

    pub fn attribute(&mut self, on_method: bool, parent: u32, constructor: u32, value: &[u8]) {
        let parent = parent << 5 | if on_method { 0 } else { 3 };
        let value = self.blob(value);
        self.row(
            super::CUSTOM_ATTRIBUTE,
            &[parent, constructor << 3 | 3, value],
        );
    }

    pub fn reference(&mut self, name: &str) {
        let name = self.string(name);
        self.row(ASSEMBLY_REF, &[0, 0, 0, name, 0, 0]);
    }

    pub fn finish(self) -> Vec<u8> {
        let mut tables = vec![0u8; 24];
        let present: u64 = (0..TABLES.len())
            .filter(|t| !self.rows[*t].is_empty())
            .fold(0, |bits, t| bits | 1 << t);
        tables[8..16].copy_from_slice(&present.to_le_bytes());
        for rows in self.rows.iter().filter(|r| !r.is_empty()) {
            tables.extend((rows.len() as u32).to_le_bytes());
        }
        for (table, rows) in self.rows.iter().enumerate() {
            for row in rows {
                for (cell, col) in row.iter().zip(TABLES[table]) {
                    match col {
                        Col::Fixed(bytes) => {
                            tables.extend(&(*cell as u64).to_le_bytes()[..(*bytes).min(8)]);
                            tables.extend(std::iter::repeat_n(0, bytes.saturating_sub(8)));
                        }
                        _ => tables.extend((*cell as u16).to_le_bytes()),
                    }
                }
            }
        }
        let streams: [(&[u8], &[u8]); 4] = [
            (b"#~\0\0", &tables),
            (b"#Strings\0\0\0\0", &self.strings),
            (b"#Blob\0\0\0", &self.blobs),
            (b"#US\0", &self.user_strings),
        ];
        // Metadata: signature, a four-byte version text, the streams.
        let mut metadata = Vec::new();
        metadata.extend(0x424a_5342u32.to_le_bytes());
        metadata.extend([0u8; 8]);
        metadata.extend(4u32.to_le_bytes());
        metadata.extend([0u8; 4]);
        metadata.extend([0, 0, streams.len() as u8, 0]);
        let heads: usize = streams.iter().map(|(name, _)| 8 + name.len()).sum();
        let mut at = metadata.len() + heads;
        for (name, bytes) in streams {
            metadata.extend((at as u32).to_le_bytes());
            metadata.extend((bytes.len() as u32).to_le_bytes());
            metadata.extend(name);
            at += bytes.len();
        }
        for (_, bytes) in streams {
            metadata.extend(bytes);
        }
        // A minimal PE with one section: the .NET header first, then the metadata.
        let mut dll = vec![0u8; 0x248];
        let mut put = |at: usize, bytes: &[u8]| dll[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x3c, &0x40u32.to_le_bytes());
        put(0x40, b"PE\0\0");
        put(0x46, &[1, 0]);
        put(0x54, &[224, 0]);
        put(0x58, &[0x0b, 0x01]);
        put(0x58 + 96 + 14 * 8, &0x1000u32.to_le_bytes());
        put(0x138 + 8, &0x0010_0000u32.to_le_bytes());
        put(0x138 + 12, &0x1000u32.to_le_bytes());
        put(0x138 + 20, &0x200u32.to_le_bytes());
        put(0x208, &(CODE_RVA + self.code.len() as u32).to_le_bytes());
        dll.extend(self.code);
        dll.extend(metadata);
        dll
    }
}

/// One-byte opcode, or 0xFE plus one byte above 0xFF; then the token: row, then table.
pub fn op(code: u16, table: usize, row: u32) -> Vec<u8> {
    let mut bytes = match code {
        0..=0xff => vec![code as u8],
        _ => vec![0xfe, code as u8],
    };
    bytes.extend([row as u8, (row >> 8) as u8, 0, table as u8]);
    bytes
}

pub const VIRTUAL: u32 = 0x40;
pub const FINAL: u32 = 0x20;
pub const NEW: u32 = 0x100;

/// A custom attribute's string: one length byte, then the text.
pub fn ser(text: &str) -> Vec<u8> {
    let mut bytes = vec![text.len() as u8];
    bytes.extend(text.as_bytes());
    bytes
}
