#!/usr/bin/env rustc
//! Selection と base64 のスタンドアロンテスト
//! macOS でも実行可能（Linux 依存なし）
//!
//! 実行: rustc tests/standalone_test.rs -o /tmp/bcon_test && /tmp/bcon_test

fn main() {
    test_base64_encode();
    test_base64_decode();
    test_base64_roundtrip();
    test_selection_normalized();
    test_selection_contains_single_row();
    test_selection_contains_multi_row();
    test_selection_contains_reversed();
    eprintln!("\n=== 全テスト通過 ===");
}

// ========== Base64 ==========

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(BASE64_TABLE[((triple >> 18) & 0x3F) as usize] as char);
        output.push(BASE64_TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            output.push(BASE64_TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(BASE64_TABLE[(triple & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in input {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(output)
}

fn test_base64_encode() {
    assert_eq!(base64_encode(b"Hello World"), "SGVsbG8gV29ybGQ=");
    assert_eq!(base64_encode(b"ab"), "YWI=");
    assert_eq!(base64_encode(b"abc"), "YWJj");
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"a"), "YQ==");
    eprintln!("[OK] base64_encode");
}

fn test_base64_decode() {
    assert_eq!(base64_decode(b"SGVsbG8gV29ybGQ=").unwrap(), b"Hello World");
    assert_eq!(base64_decode(b"YWI=").unwrap(), b"ab");
    assert_eq!(base64_decode(b"YWJj").unwrap(), b"abc");
    assert_eq!(base64_decode(b"").unwrap(), b"");
    assert_eq!(base64_decode(b"YQ==").unwrap(), b"a");
    eprintln!("[OK] base64_decode");
}

fn test_base64_roundtrip() {
    let cases: &[&[u8]] = &[
        b"Hello, World!",
        "日本語テスト".as_bytes(),
        b"\x00\x01\x02\xff\xfe",
        b"OSC 52 clipboard test",
    ];
    for input in cases {
        let encoded = base64_encode(input);
        let decoded = base64_decode(encoded.as_bytes()).unwrap();
        assert_eq!(&decoded, input, "roundtrip failed for {:?}", input);
    }
    eprintln!("[OK] base64 roundtrip");
}

// ========== Selection ==========

struct Selection {
    anchor_row: usize,
    anchor_col: usize,
    end_row: usize,
    end_col: usize,
}

impl Selection {
    fn normalized(&self) -> (usize, usize, usize, usize) {
        if (self.anchor_row, self.anchor_col) <= (self.end_row, self.end_col) {
            (self.anchor_row, self.anchor_col, self.end_row, self.end_col)
        } else {
            (self.end_row, self.end_col, self.anchor_row, self.anchor_col)
        }
    }

    fn contains(&self, row: usize, col: usize) -> bool {
        let (sr, sc, er, ec) = self.normalized();
        if row < sr || row > er {
            return false;
        }
        if row == sr && row == er {
            return col >= sc && col < ec;
        }
        if row == sr {
            return col >= sc;
        }
        if row == er {
            return col < ec;
        }
        true
    }
}

fn test_selection_normalized() {
    // 順方向
    let s = Selection {
        anchor_row: 1,
        anchor_col: 5,
        end_row: 3,
        end_col: 10,
    };
    assert_eq!(s.normalized(), (1, 5, 3, 10));

    // 逆方向
    let s = Selection {
        anchor_row: 3,
        anchor_col: 10,
        end_row: 1,
        end_col: 5,
    };
    assert_eq!(s.normalized(), (1, 5, 3, 10));

    // 同一点
    let s = Selection {
        anchor_row: 2,
        anchor_col: 3,
        end_row: 2,
        end_col: 3,
    };
    assert_eq!(s.normalized(), (2, 3, 2, 3));

    eprintln!("[OK] selection normalized");
}

fn test_selection_contains_single_row() {
    // 同一行選択: row=5, col 3..7
    let s = Selection {
        anchor_row: 5,
        anchor_col: 3,
        end_row: 5,
        end_col: 7,
    };
    assert!(!s.contains(5, 2));
    assert!(s.contains(5, 3));
    assert!(s.contains(5, 6));
    assert!(!s.contains(5, 7)); // end_col は含まない
    assert!(!s.contains(4, 5));
    assert!(!s.contains(6, 5));
    eprintln!("[OK] selection contains (single row)");
}

fn test_selection_contains_multi_row() {
    // 複数行選択: (1,5) → (3,10)
    let s = Selection {
        anchor_row: 1,
        anchor_col: 5,
        end_row: 3,
        end_col: 10,
    };

    // 開始行
    assert!(!s.contains(1, 4));
    assert!(s.contains(1, 5));
    assert!(s.contains(1, 80)); // 開始行は sc 以降すべて

    // 中間行 (全選択)
    assert!(s.contains(2, 0));
    assert!(s.contains(2, 50));

    // 終了行
    assert!(s.contains(3, 0));
    assert!(s.contains(3, 9));
    assert!(!s.contains(3, 10)); // end_col は含まない

    // 範囲外
    assert!(!s.contains(0, 5));
    assert!(!s.contains(4, 0));

    eprintln!("[OK] selection contains (multi row)");
}

fn test_selection_contains_reversed() {
    // 逆方向選択でも同じ結果
    let s = Selection {
        anchor_row: 3,
        anchor_col: 10,
        end_row: 1,
        end_col: 5,
    };
    assert!(s.contains(1, 5));
    assert!(s.contains(2, 0));
    assert!(s.contains(3, 9));
    assert!(!s.contains(3, 10));
    eprintln!("[OK] selection contains (reversed)");
}
