//! Codage hexadécimal minimal (empreintes, blobs dans l'export JSON).

const DIGITS: &[u8; 16] = b"0123456789abcdef";

pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

pub(crate) fn decode(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c: u8| char::from(c).to_digit(16).map(|d| d as u8);
    s.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip() {
        let data = [0u8, 1, 0x7f, 0x80, 0xff];
        let s = super::encode(&data);
        assert_eq!(s, "00017f80ff");
        assert_eq!(super::decode(&s).unwrap(), data);
        assert_eq!(super::decode("ABCD").unwrap(), vec![0xab, 0xcd]);
        assert!(super::decode("abc").is_none());
        assert!(super::decode("zz").is_none());
    }
}
