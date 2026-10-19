# Shiva — local asset I/O security boundary

This repository is an independent formatter/extractor. A source document, its image
references and its embedded resources are untrusted. Akula must not use Shiva inside
its privileged editor process.

## Current guard (P2-MFORMAT-03a)

`disk_image_loader(root)` and `disk_image_saver(root)` now:
- accept only relative image references (including `./image.png`) without `..`;
- reject absolute references, URI-like strings and Windows ADS-style colons;
- require source/parent paths to canonicalize inside the configured root;
- refuse to write through an existing destination symlink;
- avoid putting sensitive absolute filenames in routine logs.

This does **not** establish a secure filesystem sandbox. A concurrent attacker could
swap symlinks between the path check and open/write (TOCTOU); writable hard links
can also be problematic. A real Akula integration must pass only explicitly
authorized file handles/bytes to a separate restricted process, with no network
access, bounded memory/time/decompression and read-only source custody. The
reusable loader is a compatibility helper, not a capability security primitive.

Tests include valid round-trip, relative traversal, absolute/URL-like paths,
Unix symlinks to outside files and directories, and a protected external sentinel.
They run with `cargo test -p shiva --no-default-features --features markdown --lib`.

PDF font auto-download and implicit `fonts/` writes are a **separate P0 security
blocker**. This PR does not make PDF conversion safe or offline by itself.
