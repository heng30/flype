#!/usr/bin/env python3
"""数据文件的版本标记和清单（规则见 docs/DATA.md「数据版本标记」、core/src/datamark.rs）。

    tools/datamark.py stamp <文件> <名字> <版本> [--source 来源]   加上（或换掉）标记
    tools/datamark.py show <文件>...                                 看标记
    tools/datamark.py manifest <数据目录> [别的文件...]              写 <数据目录>/DATA-MANIFEST
    tools/datamark.py --self-test

标记是一串 `flype-data: <名字> <版本> [来源]`：
- 文本文件（TSV、英文词表）：头部一行注释 `# flype-data: ...`。第一行本来就是注释时插在第二行
  （0.4.0 的插件按第一行认旧副本，照样管用），否则插在第一行。读这些文件的代码都把 # 开头的行跳过。
- words.lm（WTLM 版本 1）：词表 1 号词就是这一串（0 号是 <s>），原来的词编号顺延，格式版本不变，
  0.4.0 的读取代码照样能读。和 core/src/wordlm.rs 的 set_data_marker 一样。
- 别的二进制文件：自己把这一串写在前 4096 字节里，以 \\n 或 \\0 结尾（这个工具不改它们）。
版本是点分隔的数字（数据日期 + 修订号，如 2026.11.14.5），逐段按数字比。

清单 DATA-MANIFEST 列出数据目录里认识的数据文件（DATA_FILES）和另外给的文件，每行
`名字<TAB>版本<TAB>sha256<TAB>来源`，装进安装包的系统数据目录（/usr/share/flype）。
没有标记的文件不进清单、报错退出；加标记以前发布的 words.lm 按字节数认（LEGACY_OFFICIAL_LM）。
"""

import hashlib
import os
import re
import struct
import sys

KEY = "flype-data:"
SCAN_BYTES = 4096
MANIFEST = "DATA-MANIFEST"
# 系统数据目录里会有的数据文件（清单按这个顺序列）。nnlm.bin 是 Flype 自己的字级神经语言模型（WTNN）
DATA_FILES = ("lexicon.tsv", "english.txt", "words.lm", "nnlm.bin", "s2t.tsv")
# 加标记以前发布的官方 words.lm：字节数 -> (版本, 来源)。和插件的 kOfficialLmSizes 一样，以后不用再加
LEGACY_OFFICIAL_LM = {
    38455852: ("2026.11.09", "lm-2026.11.09（sha256 42cb7c4a7cededa29b1a9ff105530085529fefff96535cc051341ebd51605dbe）"),
}
_NAME = re.compile(r"[\x21-\x7e]+")
_VERSION = re.compile(r"[0-9]+(?:\.[0-9]+)*")


def valid_marker(name, version, source=""):
    return (bool(_NAME.fullmatch(name)) and bool(_VERSION.fullmatch(version))
            and not any(c in source for c in "\n\r\0"))


def parse(text):
    """`... flype-data: 名字 版本 [来源]` -> (名字, 版本, 来源)，不合规矩返回 None。"""
    at = text.find(KEY)
    if at < 0:
        return None
    rest = re.split(r"[\n\r\0]", text[at + len(KEY):], maxsplit=1)[0]
    if not rest.startswith(" "):
        return None
    rest = rest.lstrip(" ")
    name, _, rest = rest.partition(" ")
    version, _, source = rest.lstrip(" ").partition(" ")
    source = source.strip()
    return (name, version, source) if valid_marker(name, version, source) else None


def body(name, version, source=""):
    return f"{KEY} {name} {version}" + (f" {source}" if source else "")


def version_key(version):
    return tuple(int(part) for part in version.split("."))


def compare_versions(a, b):
    """逐段按数字比，缺的段算 0。"""
    x, y = list(version_key(a)), list(version_key(b))
    n = max(len(x), len(y))
    x += [0] * (n - len(x))
    y += [0] * (n - len(y))
    return (x > y) - (x < y)


def scan(head):
    head = head[:SCAN_BYTES]
    if head.startswith(b"WTLM"):
        return _wtlm_marker(head)
    at = head.find(KEY.encode())
    if at < 0:
        return None
    ends = [i for i in (head.find(b"\n", at), head.find(b"\r", at), head.find(b"\0", at)) if i >= 0]
    if not ends:
        return None
    try:
        return parse(head[at:min(ends)].decode("utf-8"))
    except UnicodeDecodeError:
        return None


def _wtlm_marker(head):
    if len(head) < 17:
        return None
    version, n = struct.unpack_from("<II", head, 4)
    if version != 1 or n < 2:
        return None
    at = 17 + head[16]
    if at >= len(head):
        return None
    word = head[at + 1:at + 1 + head[at]]
    if len(word) != head[at] or not word.startswith(KEY.encode()):
        return None
    try:
        return parse(word.decode("utf-8"))
    except UnicodeDecodeError:
        return None


def read_marker(path):
    with open(path, "rb") as f:
        return scan(f.read(SCAN_BYTES))


def file_version(path):
    """(名字, 版本, 来源)：标记，或者加标记以前的官方 words.lm；都不是返回 None。"""
    marker = read_marker(path)
    if marker:
        return marker
    if os.path.basename(path) == "words.lm":
        legacy = LEGACY_OFFICIAL_LM.get(os.path.getsize(path))
        if legacy:
            return ("words.lm", legacy[0], legacy[1] + "，没有标记，按字节数认出")
    return None


def _write(path, data):
    tmp = path + ".datamark-tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def stamp_text(data, line):
    lines = data.splitlines(keepends=True)
    # 去掉头部已有的标记（只看开头的注释行）
    head = 0
    while head < len(lines) and head < 32 and lines[head].startswith(b"#"):
        head += 1
    lines = [l for i, l in enumerate(lines) if i >= head or KEY.encode() not in l]
    at = 1 if lines and lines[0].startswith(b"#") else 0
    if at == 1 and not lines[0].endswith(b"\n"):
        lines[0] += b"\n"
    lines.insert(at, line.encode("utf-8") + b"\n")
    return b"".join(lines)


def stamp_wtlm(data, marker):
    """和 core/src/wordlm.rs 的 set_data_marker 一样。"""
    import array
    raw = marker.encode("utf-8")
    if len(raw) > 255:
        raise ValueError(f"标记太长（{len(raw)} 字节，最多 255）")
    if data[:4] != b"WTLM":
        raise ValueError("不是 Flype 词级语言模型")
    version, n, m = struct.unpack_from("<III", data, 4)
    if version != 1:
        raise ValueError(f"版本 {version} 不认识")
    if n == 0:
        raise ValueError("词表是空的（没有 <s>）")
    at = 16
    words = []
    for _ in range(n):
        size = data[at]
        words.append(data[at + 1:at + 1 + size])
        at += 1 + size
    backoff = data[at:at + 4 * n]
    at += 4 * n
    starts = array.array("I", data[at:at + 4 * (n + 1)])
    at += 4 * (n + 1)
    curs = array.array("I", data[at:at + 4 * m])
    at += 4 * m
    scores = data[at:at + 4 * m]
    at += 4 * m
    if len(starts) != n + 1 or len(curs) != m or len(scores) != 4 * m or at != len(data):
        raise ValueError("文件截断或末尾有多余的字节")
    if sys.byteorder != "little":
        starts.byteswap()
        curs.byteswap()
    has = n > 1 and words[1].startswith(KEY.encode())
    if has:
        words[1] = raw
    else:
        words.insert(1, raw)
        backoff = backoff[:4] + struct.pack("<f", 0.0) + backoff[4:]
        starts.insert(1, starts[1])
        curs = array.array("I", (c + 1 if c else 0 for c in curs))
    if sys.byteorder != "little":
        starts.byteswap()
        curs.byteswap()
    out = [b"WTLM", struct.pack("<III", 1, len(words), m)]
    out.extend(bytes([len(w)]) + w for w in words)
    out += [backoff, starts.tobytes(), curs.tobytes(), scores]
    return b"".join(out)


def stamp(path, name, version, source=""):
    if not valid_marker(name, version, source):
        raise ValueError(f"标记不合规矩：名字 {name!r}、版本 {version!r}（要点分隔的数字）")
    with open(path, "rb") as f:
        data = f.read()
    text = body(name, version, source)
    if data.startswith(b"WTLM"):
        out = stamp_wtlm(data, text)
    elif b"\0" in data[:SCAN_BYTES]:
        raise ValueError(f"{path} 是不认识的二进制格式，标记要由生成它的程序写")
    else:
        out = stamp_text(data, "# " + text)
    if scan(out[:SCAN_BYTES]) != (name, version, source.strip()):
        raise ValueError(f"{path} 加了标记却读不出来（第一行太长？标记要在前 {SCAN_BYTES} 字节里）")
    _write(path, out)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def manifest(folder, extra=()):
    """写 folder/DATA-MANIFEST，返回各行。"""
    paths = [os.path.join(folder, name) for name in DATA_FILES
             if os.path.isfile(os.path.join(folder, name))]
    for path in extra:
        if os.path.basename(path) in {os.path.basename(p) for p in paths}:
            continue
        paths.append(path)
    rows = []
    for path in paths:
        found = file_version(path)
        if not found:
            raise ValueError(f"{path} 没有 flype-data 标记（tools/datamark.py stamp 加上）")
        name, version, source = found
        if name != os.path.basename(path):
            raise ValueError(f"{path} 的标记名字是 {name}，和文件名对不上")
        rows.append(f"{name}\t{version}\t{sha256(path)}\t{source}")
    if not rows:
        raise ValueError(f"{folder} 里没有数据文件")
    text = ("# Flype 数据清单（tools/datamark.py manifest 生成，规则见 docs/DATA.md「数据版本标记」）\n"
            "# 名字<TAB>版本<TAB>sha256<TAB>来源\n" + "".join(r + "\n" for r in rows))
    _write(os.path.join(folder, MANIFEST), text.encode("utf-8"))
    return rows


def self_test():
    import tempfile
    assert parse("# flype-data: lexicon.tsv 2026.11.14.5 rime-ice abc (2026-11-14)\n") == \
        ("lexicon.tsv", "2026.11.14.5", "rime-ice abc (2026-11-14)")
    assert parse("# flype-data: lexicon.tsv 2026-11-14.5") is None
    assert parse("# flype-data:lexicon.tsv 1") is None
    assert compare_versions("2026.11.9", "2026.11.10") < 0
    assert compare_versions("2026.11.14", "2026.11.14.0") == 0
    assert compare_versions("2026.11.09", "2026.11.9") == 0
    with tempfile.TemporaryDirectory() as root:
        lex = os.path.join(root, "lexicon.tsv")
        with open(lex, "w", encoding="utf-8") as f:
            f.write("# 由 tools/rime2tsv.py 生成\nni hao\t你好\t1\n")
        stamp(lex, "lexicon.tsv", "2026.11.14.5", "rime-ice abc")
        stamp(lex, "lexicon.tsv", "2026.11.14.6", "rime-ice abc")  # 再加一次是换掉
        with open(lex, encoding="utf-8") as f:
            assert f.read() == ("# 由 tools/rime2tsv.py 生成\n"
                                "# flype-data: lexicon.tsv 2026.11.14.6 rime-ice abc\nni hao\t你好\t1\n")
        # WTLM：<s>、甲、乙；<s>->甲、甲->乙 两个搭配
        lm = os.path.join(root, "words.lm")
        words = ["<s>", "甲", "乙"]
        data = b"WTLM" + struct.pack("<III", 1, 3, 2)
        data += b"".join(bytes([len(w.encode())]) + w.encode() for w in words)
        data += struct.pack("<3f", -0.5, -1.0, 0.0) + struct.pack("<4I", 0, 1, 2, 2)
        data += struct.pack("<2I", 1, 2) + struct.pack("<2f", 1.5, 2.5)
        with open(lm, "wb") as f:
            f.write(data)
        assert file_version(lm) is None
        stamp(lm, "words.lm", "2026.11.20.0", "lm-2026.11.20")
        with open(lm, "rb") as f:
            out = f.read()
        expect = b"WTLM" + struct.pack("<III", 1, 4, 2)
        marker = body("words.lm", "2026.11.20.0", "lm-2026.11.20").encode()
        expect += b"\x03<s>" + bytes([len(marker)]) + marker + b"\x03\xe7\x94\xb2\x03\xe4\xb9\x99"
        expect += struct.pack("<4f", -0.5, 0.0, -1.0, 0.0) + struct.pack("<5I", 0, 1, 1, 2, 2)
        expect += struct.pack("<2I", 2, 3) + struct.pack("<2f", 1.5, 2.5)
        assert out == expect, out
        stamp(lm, "words.lm", "2026.11.21.0")
        assert file_version(lm) == ("words.lm", "2026.11.21.0", "")
        with open(lm, "rb") as f:
            assert len(f.read()) == len(expect) - len(marker) + len(body("words.lm", "2026.11.21.0"))
        legacy = os.path.join(root, "old", "words.lm")
        os.makedirs(os.path.dirname(legacy))
        with open(legacy, "wb") as f:
            f.truncate(38455852)
        assert file_version(legacy)[1] == "2026.11.09"
        rows = manifest(root)
        assert [r.split("\t")[:2] for r in rows] == [
            ["lexicon.tsv", "2026.11.14.6"], ["words.lm", "2026.11.21.0"]]
        with open(os.path.join(root, "english.txt"), "w") as f:
            f.write("# Rime dictionary\nhello\n")
        try:
            manifest(root)
        except ValueError:
            pass
        else:
            raise AssertionError("没有标记的文件应该报错")
    print("datamark self-test ok")


def main(argv):
    if argv[:1] == ["--self-test"]:
        self_test()
        return 0
    if len(argv) >= 4 and argv[0] == "stamp":
        source = ""
        rest = argv[4:]
        if rest[:1] == ["--source"] and len(rest) == 2:
            source = rest[1]
        elif rest:
            print(__doc__, file=sys.stderr)
            return 2
        stamp(argv[1], argv[2], argv[3], source)
        return 0
    if len(argv) >= 2 and argv[0] == "show":
        status = 0
        for path in argv[1:]:
            found = file_version(path)
            if found:
                print(f"{path}\t" + "\t".join(found))
            else:
                print(f"{path}\t没有标记")
                status = 1
        return status
    if len(argv) >= 2 and argv[0] == "manifest":
        for row in manifest(argv[1], argv[2:]):
            print(row)
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError) as e:
        print(f"datamark: {e}", file=sys.stderr)
        sys.exit(1)
