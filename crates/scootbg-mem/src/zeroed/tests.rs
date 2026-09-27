use super::zeroed_bytes;

#[test]
fn small_and_empty_buffers_are_zeroed() {
    assert_eq!(zeroed_bytes(0), Some(Vec::new()));
    for len in [1, 7, 4096, 128 * 1024 - 1, 128 * 1024, 300_000] {
        let bytes = zeroed_bytes(len).unwrap();
        assert_eq!(bytes.len(), len);
        assert!(bytes.capacity() >= len);
        assert!(bytes.iter().all(|&b| b == 0), "{len}");
    }
}

#[test]
fn an_impossible_size_is_none_not_an_abort() {
    assert_eq!(zeroed_bytes(usize::MAX), None);
    assert_eq!(zeroed_bytes(isize::MAX as usize + 1), None);
}

#[test]
fn the_buffer_grows_and_frees_like_any_vec() {
    let mut bytes = zeroed_bytes(200_000).unwrap();
    bytes[199_999] = 7;
    bytes.truncate(10);
    bytes.shrink_to_fit();
    bytes.extend_from_slice(&[1; 300_000]);
    assert_eq!(bytes.len(), 300_010);
    assert_eq!(bytes[..10], [0; 10]);
}
