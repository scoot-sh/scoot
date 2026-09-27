use super::hex;

/// FIPS 180-4's examples, and NIST's million-`a` vector.
#[test]
fn the_standard_vectors() {
    assert_eq!(
        hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        hex(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

/// Every padding boundary: a tail with room for the length (55), one
/// without (56, 63), a whole block (64), and past one; checked against
/// coreutils' `sha256sum` over that many `x`s.
#[test]
fn the_padding_boundaries() {
    for (len, want) in [
        (
            55,
            "d5e285683cd4efc02d021a5c62014694958901005d6f71e89e0989fac77e4072",
        ),
        (
            56,
            "04c26261370ee7541549d16dee320c723e3fd14671e66a099afe0a377c16888e",
        ),
        (
            63,
            "75220b47218278e656f2013bb8f0c455a25eaf01e86c64924e9d48d89776d6f2",
        ),
        (
            64,
            "7ce100971f64e7001e8fe5a51973ecdfe1ced42befe7ee8d5fd6219506b5393c",
        ),
        (
            65,
            "9537c5fdf120482f7d58d25e9ed583f52c02b4e304ea814db1633ad565aed7e9",
        ),
        (
            119,
            "000b48d4edf0fa7bee3c6236ecd2785baa5db4eeb8bb54341b029e0d9fa5fb0c",
        ),
        (
            120,
            "13f05a0b594787f5ecd315edc96141bd3243203d1b7d4f0836f37308b276ba98",
        ),
        (
            128,
            "24da1b81d0b16df6428eee73c69fcb2a93c76bc6df706f0c6670fe6bfe800464",
        ),
    ] {
        assert_eq!(hex(&vec![b'x'; len]), want, "{len} bytes");
    }
}
