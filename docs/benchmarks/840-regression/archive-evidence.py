#!/usr/bin/env python3
"""Deterministically package one #840 follow-up evidence directory."""
import gzip
import hashlib
from pathlib import Path
import sys
import tarfile

root = Path(__file__).resolve().parent
name = sys.argv[1]
output = {'live-evidence': 'live-notifications.tar.gz', 'completion-evidence': 'completion.tar.gz'}[name]
source = root / name
files = sorted(p for p in source.rglob('*') if p.is_file() and p.name != 'SHA256SUMS')
assert files and all(not p.is_symlink() for p in files)
(source / 'SHA256SUMS').write_text(''.join(
    hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + str(p.relative_to(source)) + '\n' for p in files))
with (root / output).open('wb') as raw:
    with gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w') as archive:
            for p in sorted(source.rglob('*')):
                if p.is_file():
                    info = archive.gettarinfo(str(p), arcname=str(p.relative_to(root)))
                    info.uid = info.gid = 0
                    info.uname = info.gname = ''
                    info.mtime = 0
                    with p.open('rb') as stream:
                        archive.addfile(info, stream)
files = sorted(p for p in root.iterdir() if p.is_file() and p.name != 'SHA256SUMS')
(root / 'SHA256SUMS').write_text(''.join(
    hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + p.name + '\n' for p in files))
print(output, (root / output).stat().st_size, 'bytes')
