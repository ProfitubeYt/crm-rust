"""Turn the shipped Excel template into one owner's ready-to-open workbook.

The template is blank: its hidden ``_sync`` sheet names no workbook, server or
owner, so ``ExcelSync.StartSync`` never makes a request. ``configure_workbook``
fills in exactly what the workbook's own VBA (tools/excel_sync/ExcelSync.bas)
reads from that sheet, and nothing else:

    B1  workbook id  StartSync. A blank one is replaced in Excel by a random id
                     the server has never heard of.
    B5  server url   ConfigureSharedProfile; must satisfy ValidServerUrl and is
                     compared as text with the address in the local state.
    B7  owner id     ConfigureSharedProfile; CheckIdentity compares it with the
                     snapshot's ``user_id`` before any change is sent.
    B8  "1"          shared mode. SharedProfile() is what makes the workbook ask
                     for the owner's password; another value stops StartSync.
    B6  blank        the legacy in-file key. A workbook never carries a key.
    B9  blank        the status line of whoever saved the template last.

B2 to B4 and B10 are layout facts of the template and stay as they are. The
function takes no password, token or workbook name, so none can reach the
file. Every other ZIP entry, ``xl/vbaProject.bin`` included, is copied as the
raw bytes of the template; the rewritten sheet keeps its place, name,
compression method and timestamp.
"""
from __future__ import annotations

import io
import posixpath
import re
import struct
import uuid
import zipfile
import zlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional
from xml.parsers import expat
from xml.sax.saxutils import escape

TEMPLATE_PATH = Path(__file__).resolve().parents[1] / "assets" / "excel_sync_template.xlsm"

SYNC_SHEET = "_sync"
# Column A of the _sync sheet as tools/build_sync_workbook.py writes it. The
# VBA addresses these rows by number, so a template laid out differently is
# refused rather than filled in the wrong places.
SYNC_LABELS = ("workbook_id", "protocol", "data_sheet", "source_version", "server_url",
               "shared_token", "owner_id", "shared_mode", "status", "active_view")
SHARED_MODE = "1"
# Cells that identify a workbook or unlock it. A template that has any of them
# is somebody's configured workbook, possibly with their receipts in it.
_MUST_BE_BLANK = {"B1": "a workbook id", "B5": "a server address", "B6": "a connection key", "B7": "an owner"}

# ExcelSync.bas, ValidServerUrl: the pattern verbatim (VBScript.RegExp with
# IgnoreCase). tests/test_excel_workbook_builder.py keeps the two in step.
VBA_SERVER_URL_PATTERN = r"^https://[a-z0-9.-]+(:[0-9]+)?$|^http://(127\.0\.0\.1|localhost)(:[0-9]+)?$"
# ASCII keeps letters such as the Kelvin sign out of a-z; fullmatch keeps "$"
# from accepting a trailing line feed, which VBScript does not.
_VBA_SERVER_URL = re.compile(VBA_SERVER_URL_PATTERN, re.IGNORECASE | re.ASCII)
_DEFAULT_PORTS = {"https": 443, "http": 80}

_MAIN_NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
_DOC_REL_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
_PKG_REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
_SHARED_STRINGS_TYPE = _DOC_REL_NS + "/sharedStrings"
_WORKBOOK, _WORKBOOK_RELS, _VBA_PROJECT = "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "xl/vbaProject.bin"

_EOCD = struct.Struct("<4s4H2LH")      # end of central directory
_CENTRAL = struct.Struct("<4s6H3L5H2L")  # central directory record, before the name
_LOCAL = struct.Struct("<4s5H3L2H")    # local file header, before the name
_ENCRYPTED, _DATA_DESCRIPTOR = 0x0001, 0x0008


class InvalidServerUrl(ValueError):
    """The workbook's own VBA would refuse this address. The text never repeats the address."""


class WorkbookTemplateError(Exception):
    """The template is not what this module relies on; no workbook is produced."""


def normalize_server_url(value: object) -> str:
    """The one spelling of a server address that is written into workbooks.

    Accepted is what ``ValidServerUrl`` accepts, with or without the single
    trailing slash a base URL carries: ``https://host[:port]``, and plain http
    only for 127.0.0.1 and localhost. No path, query, fragment, user name,
    spaces or non-ASCII host. Scheme and host are lower-cased and a default
    port is dropped, because the workbook compares the address as text with
    the one in its local state and refuses to start when they differ.
    """
    if not isinstance(value, str) or not value:
        raise InvalidServerUrl("an address is required")
    text = value[:-1] if value.endswith("/") else value
    if _VBA_SERVER_URL.fullmatch(text) is None:
        raise InvalidServerUrl(
            "it must be https://host or https://host:port with nothing after it "
            "(http only for 127.0.0.1 and localhost)")
    # The pattern leaves nothing to interpret: no "@", "/", "?" or "[" can be here.
    scheme, _, authority = text.lower().partition("://")
    host, _, port = authority.partition(":")
    labels = host.split(".")
    if len(host) > 253 or any(not 0 < len(label) < 64 or label[0] == "-" or label[-1] == "-" for label in labels):
        raise InvalidServerUrl("the host name is not valid")
    if not port:
        return f"{scheme}://{host}"
    if len(port) > 5 or not 1 <= int(port) <= 65535:
        raise InvalidServerUrl("the port must be between 1 and 65535")
    return f"{scheme}://{host}" if int(port) == _DEFAULT_PORTS[scheme] else f"{scheme}://{host}:{int(port)}"


def configure_workbook(template: bytes, *, workbook_id: str, server_url: str, owner_id: int) -> bytes:
    """The template with its ``_sync`` sheet filled in for this workbook, server and owner.

    The same arguments give the same bytes. ValueError means the caller passed
    something that must not be written; WorkbookTemplateError means the
    template cannot be trusted to produce a working file.
    """
    try:
        canonical_id = str(uuid.UUID(workbook_id))
    except (ValueError, AttributeError, TypeError):
        raise ValueError("workbook_id must be a UUID") from None
    if canonical_id != workbook_id:
        raise ValueError("workbook_id must be a lower-case hyphenated UUID")
    if normalize_server_url(server_url) != server_url:
        raise ValueError("server_url must come from normalize_server_url")
    if isinstance(owner_id, bool) or not isinstance(owner_id, int) or owner_id < 1:
        raise ValueError("owner_id must be a positive integer")
    wanted = {"B1": workbook_id, "B5": server_url, "B6": "", "B7": str(owner_id), "B8": SHARED_MODE, "B9": ""}
    if not isinstance(template, (bytes, bytearray)) or not template:
        raise WorkbookTemplateError("the template is empty")
    template = bytes(template)
    try:
        with zipfile.ZipFile(io.BytesIO(template)) as archive:
            names = archive.namelist()
            if len(set(names)) != len(names):
                raise WorkbookTemplateError("the template has two parts with one name")
            if _VBA_PROJECT not in names or not archive.getinfo(_VBA_PROJECT).file_size:
                raise WorkbookTemplateError("the template has no VBA project")
            sheets = _workbook_sheets(archive.read(_WORKBOOK))
            relations = _relationships(archive.read(_WORKBOOK_RELS))
            if SYNC_SHEET not in sheets or sheets[SYNC_SHEET] not in relations:
                raise WorkbookTemplateError(f"the template has no {SYNC_SHEET} sheet")
            part = relations[sheets[SYNC_SHEET]][1]
            if part not in names:
                raise WorkbookTemplateError(f"the {SYNC_SHEET} sheet part {part} is missing")

            def shared_strings() -> list[str]:
                targets = [target for kind, target in relations.values() if kind == _SHARED_STRINGS_TYPE]
                if len(targets) != 1 or targets[0] not in names:
                    raise WorkbookTemplateError("the template refers to shared strings it does not have")
                return _shared_strings(targets[0], archive.read(targets[0]))

            sheet = _configure_sync_sheet(part, archive.read(part), wanted, set(sheets), shared_strings)
        result = _replace_entry(template, part, sheet)
        _check_result(template, result, part, sheet)
        return result
    except (zipfile.BadZipFile, zipfile.LargeZipFile, zlib.error, struct.error, expat.ExpatError,
            UnicodeDecodeError, KeyError, EOFError, NotImplementedError, RuntimeError) as exc:
        raise WorkbookTemplateError(f"the template cannot be read: {exc!r}") from exc


# --- XML ------------------------------------------------------------------

def _qualified(namespace: str, name: str) -> str:
    return f"{namespace}\n{name}"


def _no_doctype(*_args: object) -> None:
    raise WorkbookTemplateError("a template part declares a document type")


def _parser(part: str, xml: bytes) -> expat.XMLParserType:
    """Namespace-aware, without DTDs. Offsets are only meaningful for UTF-8."""
    xml.decode("utf-8")
    declared = re.match(rb"(?:\xef\xbb\xbf)?<\?xml[^>]*?encoding\s*=\s*[\"']([A-Za-z0-9._-]+)[\"']", xml)
    if declared and declared.group(1).lower() not in (b"utf-8", b"utf8"):
        raise WorkbookTemplateError(f"{part} is not UTF-8")
    parser = expat.ParserCreate(namespace_separator="\n")
    parser.buffer_text = True
    parser.StartDoctypeDeclHandler = _no_doctype
    return parser


def _workbook_sheets(xml: bytes) -> dict[str, str]:
    """Sheet name to relationship id, from xl/workbook.xml."""
    sheets: dict[str, str] = {}
    parser = _parser(_WORKBOOK, xml)

    def start(tag: str, attrs: dict[str, str]) -> None:
        if tag == _qualified(_MAIN_NS, "sheet"):
            name, relation = attrs.get("name"), attrs.get(_qualified(_DOC_REL_NS, "id"))
            if not name or not relation or name in sheets:
                raise WorkbookTemplateError("the template's sheet list is not usable")
            sheets[name] = relation

    parser.StartElementHandler = start
    parser.Parse(xml, True)
    return sheets


def _relationships(xml: bytes) -> dict[str, tuple[str, str]]:
    """Relationship id to (type, part name), from xl/_rels/workbook.xml.rels."""
    relations: dict[str, tuple[str, str]] = {}
    parser = _parser(_WORKBOOK_RELS, xml)

    def start(tag: str, attrs: dict[str, str]) -> None:
        if tag != _qualified(_PKG_REL_NS, "Relationship") or attrs.get("TargetMode") == "External":
            return
        target = attrs.get("Target") or ""
        # Targets are relative to xl/ unless they start at the package root.
        name = target[1:] if target.startswith("/") else posixpath.normpath(posixpath.join("xl", target))
        relations[attrs.get("Id") or ""] = (attrs.get("Type") or "", name)

    parser.StartElementHandler = start
    parser.Parse(xml, True)
    return relations


def _shared_strings(part: str, xml: bytes) -> list[str]:
    strings: list[str] = []
    path: list[str] = []
    parser = _parser(part, xml)
    item, text, phonetic = _qualified(_MAIN_NS, "si"), _qualified(_MAIN_NS, "t"), _qualified(_MAIN_NS, "rPh")

    def start(tag: str, _attrs: dict[str, str]) -> None:
        path.append(tag)
        if tag == item:
            strings.append("")

    def data(chunk: str) -> None:
        if path and path[-1] == text and item in path and phonetic not in path:
            strings[-1] += chunk

    parser.StartElementHandler, parser.CharacterDataHandler = start, data
    parser.EndElementHandler = lambda _tag: path.pop()
    parser.Parse(xml, True)
    return strings


@dataclass
class _Cell:
    ref: str
    start: int                      # byte span of the whole element in the part
    end: int = 0
    attrs: dict[str, str] = field(default_factory=dict)
    value: str = ""                 # text of <v>
    inline: str = ""                # text of <is>
    formula: bool = False

    def facts(self) -> tuple:
        return self.attrs, self.value, self.inline, self.formula


def _tag_end(xml: bytes, at: int) -> int:
    """Index just past the tag that starts at ``at``; ``>`` may occur inside a quoted value."""
    quote = 0
    for index in range(at, len(xml)):
        byte = xml[index]
        if quote:
            quote = 0 if byte == quote else quote
        elif byte in (0x22, 0x27):
            quote = byte
        elif byte == 0x3E:
            return index + 1
    raise WorkbookTemplateError("a tag of the template is not closed")


def _scan_cells(part: str, xml: bytes) -> dict[str, _Cell]:
    """Every cell of a worksheet part by its reference, with its byte span."""
    cells: dict[str, _Cell] = {}
    path: list[str] = []
    current: list[_Cell] = []
    parser = _parser(part, xml)
    names = {key: _qualified(_MAIN_NS, key) for key in ("worksheet", "sheetData", "row", "c", "v", "f", "is", "t", "rPh")}
    cell_path = [names["worksheet"], names["sheetData"], names["row"], names["c"]]

    def start(tag: str, attrs: dict[str, str]) -> None:
        path.append(tag)
        if path == cell_path:
            ref = attrs.get("r") or ""
            if not re.fullmatch(r"[A-Z]{1,3}[1-9][0-9]{0,6}", ref) or ref in cells:
                raise WorkbookTemplateError(f"{part} has a cell without a usable reference")
            current.append(_Cell(ref=ref, start=parser.CurrentByteIndex, attrs=dict(attrs)))
        elif current and tag == names["f"]:
            current[0].formula = True

    def data(chunk: str) -> None:
        if not current:
            return
        if path[4:] == [names["v"]]:
            current[0].value += chunk
        elif path[-1] == names["t"] and path[4:5] == [names["is"]] and names["rPh"] not in path:
            current[0].inline += chunk

    def end(_tag: str) -> None:
        if path == cell_path:
            cell = current.pop()
            cell.end = _tag_end(xml, cell.start)
            if xml[cell.end - 2:cell.end] != b"/>":
                # Not <c .../>: expat is now at the start of its </c>.
                closing = parser.CurrentByteIndex
                if xml[closing:closing + 2] != b"</":
                    raise WorkbookTemplateError(f"cell {cell.ref} of {part} cannot be located")
                cell.end = _tag_end(xml, closing)
            cells[cell.ref] = cell
        path.pop()

    parser.StartElementHandler, parser.CharacterDataHandler, parser.EndElementHandler = start, data, end
    parser.Parse(xml, True)
    return cells


def _cell_text(cell: Optional[_Cell], shared_strings: Callable[[], list[str]]) -> str:
    if cell is None:
        return ""
    kind = cell.attrs.get("t", "n")
    if kind == "inlineStr":
        return cell.inline
    if kind == "s":
        strings = shared_strings()
        if not cell.value.isdigit() or int(cell.value) >= len(strings):
            raise WorkbookTemplateError(f"cell {cell.ref} points at a shared string that does not exist")
        return strings[int(cell.value)]
    return cell.value


def _cell_xml(xml: bytes, like: _Cell, ref: str, style: Optional[str], value: str) -> bytes:
    """A text cell spelled with the namespace prefix its neighbour ``like`` uses."""
    tag = re.match(rb"<((?:[A-Za-z_][\w.-]*:)?)c[\s/>]", xml[like.start:like.end])
    if tag is None or (style is not None and not style.isdigit()):
        raise WorkbookTemplateError(f"cell {like.ref} of the {SYNC_SHEET} sheet is not written as expected")
    prefix = tag.group(1).decode("ascii")
    head = f'<{prefix}c r="{ref}"' + (f' s="{style}"' if style is not None else "")
    if not value:
        return f"{head}/>".encode("utf-8")
    return (f'{head} t="inlineStr"><{prefix}is><{prefix}t>{escape(value)}</{prefix}t></{prefix}is></{prefix}c>'
            ).encode("utf-8")


def _configure_sync_sheet(part: str, xml: bytes, wanted: dict[str, str], sheet_names: set[str],
                          shared_strings: Callable[[], list[str]]) -> bytes:
    cells = _scan_cells(part, xml)

    def text(ref: str) -> str:
        return _cell_text(cells.get(ref), shared_strings)

    for row, label in enumerate(SYNC_LABELS, 1):
        if text(f"A{row}") != label:
            raise WorkbookTemplateError(f"{SYNC_SHEET}!A{row} is not {label!r}: the template has another layout")
    for ref, meaning in _MUST_BE_BLANK.items():
        if text(ref).strip():
            raise WorkbookTemplateError(f"the template already has {meaning} in {SYNC_SHEET}!{ref}")
    if text("B3") not in sheet_names:
        raise WorkbookTemplateError(f"{SYNC_SHEET}!B3 does not name a sheet of the template")

    edits: list[tuple[int, int, bytes]] = []
    for ref, value in wanted.items():
        cell, label = cells.get(ref), cells["A" + ref[1:]]
        if cell is None:
            if value:  # column B directly follows the label in column A
                edits.append((label.end, label.end, _cell_xml(xml, label, ref, None, value)))
            continue
        if cell.formula or set(cell.attrs) - {"r", "s", "t"}:
            raise WorkbookTemplateError(f"{SYNC_SHEET}!{ref} is not a plain cell")
        if text(ref) != value:
            edits.append((cell.start, cell.end, _cell_xml(xml, cell, ref, cell.attrs.get("s"), value)))
    result = bytearray(xml)
    for start, end, replacement in sorted(edits, reverse=True):
        result[start:end] = replacement
    result = bytes(result)

    # Whatever the template looked like, hand out only a sheet that reads back
    # as intended and in which no other cell changed.
    after = _scan_cells(part, result)
    for ref, value in wanted.items():
        if _cell_text(after.get(ref), shared_strings) != value:
            raise WorkbookTemplateError(f"{SYNC_SHEET}!{ref} could not be set")
    untouched = {ref: cell.facts() for ref, cell in cells.items() if ref not in wanted}
    if untouched != {ref: cell.facts() for ref, cell in after.items() if ref not in wanted}:
        raise WorkbookTemplateError(f"configuring the {SYNC_SHEET} sheet would change other cells")
    return result


# --- ZIP ------------------------------------------------------------------

def _replace_entry(archive: bytes, name: str, content: bytes) -> bytes:
    """The archive with one entry's content replaced and every other byte of every other entry kept."""
    end_at = archive.rfind(b"PK\x05\x06")
    if end_at < 0 or end_at + _EOCD.size > len(archive):
        raise WorkbookTemplateError("the template has no ZIP directory")
    _sig, disk, directory_disk, here, total, directory_size, directory_at, comment_size = _EOCD.unpack_from(archive, end_at)
    if (disk or directory_disk or here != total or total == 0xFFFF or directory_at == 0xFFFFFFFF
            or end_at + _EOCD.size + comment_size != len(archive) or directory_at + directory_size != end_at):
        raise WorkbookTemplateError("the template's ZIP layout is not supported")

    records = []  # [directory offset, directory record size, local header offset, name]
    at = directory_at
    for _ in range(total):
        fields = _CENTRAL.unpack_from(archive, at)
        if fields[0] != b"PK\x01\x02" or fields[3] & _ENCRYPTED:
            raise WorkbookTemplateError("the template's ZIP directory is not supported")
        size = _CENTRAL.size + fields[10] + fields[11] + fields[12]
        records.append((at, size, fields[16], archive[at + _CENTRAL.size:at + _CENTRAL.size + fields[10]]))
        at += size
    order = sorted(records, key=lambda record: record[2])
    if at != end_at or not order or order[0][2] != 0 or len({record[2] for record in order}) != len(order):
        raise WorkbookTemplateError("the template's ZIP layout is not supported")

    body = bytearray()
    moved: dict[int, int] = {}
    sizes: Optional[tuple[int, int, int]] = None  # CRC, packed and plain size of the new content
    wanted = name.encode("utf-8")
    for index, (_directory_at, _size, local_at, entry_name) in enumerate(order):
        local_end = order[index + 1][2] if index + 1 < len(order) else directory_at
        signature, version, flags, method, time, date, _crc, _packed, _plain, name_size, extra_size = \
            _LOCAL.unpack_from(archive, local_at)
        name_at = local_at + _LOCAL.size
        data_at = name_at + name_size + extra_size
        if signature != b"PK\x03\x04" or data_at > local_end or archive[name_at:name_at + name_size] != entry_name:
            raise WorkbookTemplateError("the template's ZIP entries are not supported")
        moved[local_at] = len(body)
        if entry_name != wanted:
            body += archive[local_at:local_end]  # header, data and any descriptor, as they are
            continue
        if method == zipfile.ZIP_DEFLATED:
            deflate = zlib.compressobj(6, zlib.DEFLATED, -15)
            packed = deflate.compress(content) + deflate.flush()
        elif method == zipfile.ZIP_STORED:
            packed = content
        else:
            raise WorkbookTemplateError(f"{name} uses a compression this module cannot write")
        sizes = (zlib.crc32(content), len(packed), len(content))
        # The sizes go into the header itself, so no data descriptor follows.
        body += _LOCAL.pack(signature, version, flags & ~_DATA_DESCRIPTOR, method, time, date, *sizes,
                            name_size, extra_size)
        body += archive[name_at:data_at] + packed
    if sizes is None:
        raise WorkbookTemplateError(f"the template has no {name}")

    directory = bytearray()
    for record_at, size, local_at, entry_name in records:
        record = bytearray(archive[record_at:record_at + size])
        fields = list(_CENTRAL.unpack_from(record))
        fields[16] = moved[local_at]
        if entry_name == wanted:
            fields[3] &= ~_DATA_DESCRIPTOR
            fields[7], fields[8], fields[9] = sizes
        _CENTRAL.pack_into(record, 0, *fields)
        directory += record
    end = bytearray(archive[end_at:])
    _EOCD.pack_into(end, 0, b"PK\x05\x06", 0, 0, total, total, len(directory), len(body), comment_size)
    return bytes(body + directory + end)


def _check_result(template: bytes, result: bytes, part: str, content: bytes) -> None:
    """Read the result back as Excel would: same entries, only ``part`` different."""
    with zipfile.ZipFile(io.BytesIO(template)) as before, zipfile.ZipFile(io.BytesIO(result)) as after:
        old, new = before.infolist(), after.infolist()
        if [item.filename for item in old] != [item.filename for item in new]:
            raise WorkbookTemplateError("the configured workbook lost or reordered parts")
        for was, now in zip(old, new):
            same = (was.date_time, was.compress_type, was.external_attr, was.extra, was.comment) == \
                   (now.date_time, now.compress_type, now.external_attr, now.extra, now.comment)
            if not same or after.read(now) != (content if was.filename == part else before.read(was)):
                raise WorkbookTemplateError(f"{was.filename} would not survive configuration")
