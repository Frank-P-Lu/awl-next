use std::collections::BTreeMap;

pub(super) struct Pdf<'a> {
    bytes: &'a [u8],
    pub objects: BTreeMap<u32, Object<'a>>,
    pub startxref: usize,
}

pub(super) struct Object<'a> {
    pub id: u32,
    pub offset: usize,
    pub body: &'a [u8],
}

impl<'a> Pdf<'a> {
    pub fn parse(bytes: &'a [u8]) -> Self {
        assert!(
            bytes.starts_with(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n"),
            "PDF 1.7 binary header"
        );
        assert!(bytes.ends_with(b"%%EOF\n"), "exact EOF marker");
        let start_marker = rfind(bytes, b"startxref\n").expect("startxref marker");
        let number_start = start_marker + b"startxref\n".len();
        let number_end = bytes[number_start..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| number_start + i)
            .unwrap();
        let startxref = decimal(&bytes[number_start..number_end]);
        assert_eq!(&bytes[startxref..startxref + 5], b"xref\n");
        let trailer = find_from(bytes, b"trailer\n", startxref).expect("trailer");
        let xref = std::str::from_utf8(&bytes[startxref..trailer]).unwrap();
        let mut lines = xref.lines();
        assert_eq!(lines.next(), Some("xref"));
        let section = lines.next().unwrap().split_whitespace().collect::<Vec<_>>();
        assert_eq!(
            section[0], "0",
            "one classic xref subsection from object zero"
        );
        let count = section[1].parse::<usize>().unwrap();
        let entries = lines.collect::<Vec<_>>();
        assert_eq!(entries.len(), count);
        assert_eq!(entries[0], "0000000000 65535 f ");
        let mut offsets = vec![0usize];
        for entry in &entries[1..] {
            assert_eq!(entry.len(), 19, "ten-digit classic xref entry: {entry:?}");
            assert_eq!(&entry[10..], " 00000 n ");
            offsets.push(entry[..10].parse().unwrap());
        }

        let trailer_text = std::str::from_utf8(&bytes[trailer..start_marker]).unwrap();
        assert_eq!(
            trailer_text,
            format!("trailer\n<< /Size {count} /Root 1 0 R >>\n"),
            "fixed trailer"
        );
        let mut objects = BTreeMap::new();
        for id in 1..count {
            let offset = offsets[id];
            let next = if id + 1 < count {
                offsets[id + 1]
            } else {
                startxref
            };
            let header = format!("{id} 0 obj\n");
            assert_eq!(
                &bytes[offset..offset + header.len()],
                header.as_bytes(),
                "xref offset {id}"
            );
            let framed = &bytes[offset + header.len()..next];
            assert!(framed.ends_with(b"\nendobj\n"), "object {id} framing");
            let body = &framed[..framed.len() - b"\nendobj\n".len()];
            let object = Object {
                id: id as u32,
                offset,
                body,
            };
            object.validate_stream_length();
            objects.insert(id as u32, object);
        }
        assert_eq!(objects.len() + 1, count);
        Self {
            bytes,
            objects,
            startxref,
        }
    }

    pub fn object(&self, id: u32) -> &Object<'a> {
        self.objects
            .get(&id)
            .unwrap_or_else(|| panic!("missing object {id}"))
    }

    pub fn page_ids(&self) -> Vec<u32> {
        refs_in_array(self.object(2).text(), "/Kids [")
    }

    pub fn page_streams(&self) -> Vec<&'a [u8]> {
        self.page_ids()
            .iter()
            .map(|id| {
                let content = reference(self.object(*id).text(), "/Contents ");
                self.object(content).stream().unwrap()
            })
            .collect()
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

impl<'a> Object<'a> {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(self.body).into_owned()
    }

    pub fn stream(&self) -> Option<&'a [u8]> {
        let marker = find(self.body, b"\nstream\n")?;
        let start = marker + b"\nstream\n".len();
        let length = stream_length(&self.body[..marker]);
        Some(&self.body[start..start + length])
    }

    fn validate_stream_length(&self) {
        let Some(marker) = find(self.body, b"\nstream\n") else {
            return;
        };
        let length = stream_length(&self.body[..marker]);
        let start = marker + b"\nstream\n".len();
        let end = start + length;
        assert!(end <= self.body.len(), "object {} stream overrun", self.id);
        assert_eq!(
            &self.body[end..],
            b"\nendstream",
            "object {} exact /Length",
            self.id
        );
    }
}

pub(super) fn reference(text: String, marker: &str) -> u32 {
    let start = text.find(marker).unwrap() + marker.len();
    text[start..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

pub(super) fn refs_in_array(text: String, marker: &str) -> Vec<u32> {
    let start = text.find(marker).unwrap() + marker.len();
    let end = text[start..].find(']').unwrap() + start;
    let words = text[start..end].split_whitespace().collect::<Vec<_>>();
    assert_eq!(words.len() % 3, 0);
    words
        .chunks_exact(3)
        .map(|chunk| {
            assert_eq!(&chunk[1..], &["0", "R"]);
            chunk[0].parse().unwrap()
        })
        .collect()
}

pub(super) fn decode_utf16_hex(hex: &str) -> String {
    assert_eq!(hex.len() % 4, 0);
    let mut units = hex
        .as_bytes()
        .chunks_exact(4)
        .map(|chunk| u16::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    if units.first() == Some(&0xfeff) {
        units.remove(0);
    }
    String::from_utf16(&units).unwrap()
}

pub(super) fn hex_value_after<'a>(text: &'a str, marker: &str) -> &'a str {
    let start = text.find(marker).unwrap() + marker.len();
    let end = text[start..].find('>').unwrap() + start;
    &text[start..end]
}

fn stream_length(dict: &[u8]) -> usize {
    let marker = b"/Length ";
    let start = find(dict, marker).expect("stream /Length") + marker.len();
    let end = dict[start..]
        .iter()
        .position(|b| !b.is_ascii_digit())
        .map(|i| start + i)
        .unwrap_or(dict.len());
    decimal(&dict[start..end])
}

fn decimal(bytes: &[u8]) -> usize {
    std::str::from_utf8(bytes).unwrap().parse().unwrap()
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_from(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    find(&bytes[from..], needle).map(|i| from + i)
}

fn rfind(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .rposition(|window| window == needle)
}
