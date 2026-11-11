"""The configured workbook file: what is written, and what must stay as shipped.

Nothing here executes VBA. The checks read the archive the way Excel would
find it and pin the builder to the workbook's own sources in tools/excel_sync.
"""
from __future__ import annotations

import hashlib
import inspect
import io
import re
import struct
import warnings
import zipfile
from pathlib import Path
from typing import Callable, Optional
from xml.etree import ElementTree as ET

import openpyxl
import pytest

from services import excel_workbook_builder as builder

BACKEND = Path(__file__).resolve().parents[1]
VBA = (BACKEND / "tools/excel_sync/ExcelSync.bas").read_text("utf-8")
TEMPLATE = builder.TEMPLATE_PATH.read_bytes()
MAIN = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
WORKBOOK = "0f8fad5b-d9cb-469f-a165-70867728950e"
SERVER = "https://sync.example.test:8443"
OWNER = 42
EMPTY = '<s:c r="{ref}" s="200" t="n" />'
TEXT = '<s:c r="{ref}" s="200" t="inlineStr"><s:is><s:t>{value}</s:t></s:is></s:c>'


def configure(template: bytes = TEMPLATE, **overrides) -> bytes:
    arguments = {"workbook_id": WORKBOOK, "server_url": SERVER, "owner_id": OWNER, **overrides}
    return builder.configure_workbook(template, **arguments)


def parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {info.filename: archive.read(info) for info in archive.infolist()}


def sync_part(data: bytes) -> str:
    """The part behind the _sync sheet, found without the builder's own reader."""
    content = parts(data)
    workbook = ET.fromstring(content["xl/workbook.xml"])
    relation = next(sheet.get(f"{{{REL}}}id") for sheet in workbook.iter(f"{{{MAIN}}}sheet") if sheet.get("name") == "_sync")
    target = next(item.get("Target") for item in ET.fromstring(content["xl/_rels/workbook.xml.rels"])
                  if item.get("Id") == relation)
    return target[1:] if target.startswith("/") else "xl/" + target


def sync_cells(data: bytes) -> dict[str, Optional[str]]:
    """Text of every _sync cell, read with ElementTree (inline and shared strings)."""
    content = parts(data)
    shared = []
    if "xl/sharedStrings.xml" in content:
        shared = ["".join(node.text or "" for node in item.iter(f"{{{MAIN}}}t"))
                  for item in ET.fromstring(content["xl/sharedStrings.xml"])]
    cells = {}
    for cell in ET.fromstring(content[sync_part(data)]).iter(f"{{{MAIN}}}c"):
        value = cell.find(f"{{{MAIN}}}v")
        if cell.get("t") == "inlineStr":
            cells[cell.get("r")] = "".join(node.text or "" for node in cell.iter(f"{{{MAIN}}}t"))
        elif cell.get("t") == "s":
            cells[cell.get("r")] = shared[int(value.text)]
        else:
            cells[cell.get("r")] = value.text if value is not None else None
    return cells


def raw_entries(data: bytes) -> dict[str, bytes]:
    """Every entry exactly as stored: local header, name, extra field and packed bytes."""
    entries = {}
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for info in archive.infolist():
            name_size, extra_size = struct.unpack_from("<2H", data, info.header_offset + 26)
            end = info.header_offset + 30 + name_size + extra_size + info.compress_size
            entries[info.filename] = data[info.header_offset:end]
    return entries


def entry_facts(data: bytes) -> list[tuple]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return [(info.filename, info.date_time, info.compress_type, info.create_system, info.create_version,
                 info.extract_version, info.flag_bits, info.internal_attr, info.external_attr, info.extra,
                 info.comment) for info in archive.infolist()]


class Unseekable:
    """A stream zipfile can only append to, so it writes data descriptors."""

    def __init__(self) -> None:
        self.buffer = io.BytesIO()

    def write(self, data: bytes) -> int:
        return self.buffer.write(data)

    def flush(self) -> None:
        pass


def repack(data: bytes = TEMPLATE, replace: Optional[dict[str, Optional[bytes]]] = None, *,
           add: Optional[dict[str, bytes]] = None, method: Optional[int] = None, seekable: bool = True) -> bytes:
    """The archive written anew: parts replaced, dropped (None) or added."""
    replace = replace or {}
    target = io.BytesIO() if seekable else Unseekable()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(target, "w") as result:
        for info in source.infolist():
            content = replace.get(info.filename, source.read(info))
            if content is None:
                continue
            fresh = zipfile.ZipInfo(info.filename, info.date_time)
            fresh.compress_type = info.compress_type if method is None else method
            fresh.external_attr = info.external_attr
            result.writestr(fresh, content)
        for name, content in (add or {}).items():
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")  # a duplicate name is the point of one case
                result.writestr(zipfile.ZipInfo(name, (2026, 9, 24, 3, 41, 36)), content)
    return target.getvalue() if seekable else target.buffer.getvalue()


def with_sheet(change: Callable[[str], str], data: bytes = TEMPLATE) -> bytes:
    name = sync_part(data)
    before = parts(data)[name].decode("utf-8")
    after = change(before)
    assert after != before, "the template no longer contains what this case edits"
    return repack(data, {name: after.encode("utf-8")})


def filled(ref: str, value: str) -> Callable[[str], str]:
    return lambda xml: xml.replace(EMPTY.format(ref=ref), TEXT.format(ref=ref, value=value))


# --- what is written ---------------------------------------------------------

def test_the_shipped_template_is_blank_and_the_configured_file_is_not():
    blank = sync_cells(TEMPLATE)
    assert [blank[f"A{row}"] for row in range(1, 11)] == list(builder.SYNC_LABELS)
    assert [blank[ref] for ref in ("B1", "B5", "B6", "B7")] == [None, None, None, None]

    book = openpyxl.load_workbook(io.BytesIO(configure()), keep_vba=True)
    sheet = book["_sync"]
    assert [(sheet.cell(row, 1).value, sheet.cell(row, 2).value) for row in range(1, 11)] == [
        ("workbook_id", WORKBOOK), ("protocol", "1"), ("data_sheet", "_transactions"), ("source_version", "1"),
        ("server_url", SERVER), ("shared_token", None), ("owner_id", "42"), ("shared_mode", "1"),
        ("status", None), ("active_view", None)]
    # Text cells, as the VBA compares them: an owner id must not come back as 42.0.
    assert all(sheet.cell(row, 2).number_format == "@" for row in range(1, 11))
    assert sheet.sheet_state == "veryHidden" and book.vba_archive is not None
    assert book.sheetnames == openpyxl.load_workbook(io.BytesIO(TEMPLATE), keep_vba=True).sheetnames


def test_the_cells_written_are_the_cells_the_vba_reads():
    # A change to any of these lines means the builder has to be looked at again.
    for line in (
        'mWorkbookId = CStr(cfg.Range("B1").Value2)',
        'url = Trim$(TextValue(cfg.Range("B5").Value2))',
        'owner = Trim$(TextValue(cfg.Range("B7").Value2))',
        'If TextValue(cfg.Range("B8").Value2) <> "1" Then Err.Raise IDENTITY_ERROR, , "Shared connection is disabled"',
        'If Len(TextValue(cfg.Range("B6").Value2)) > 0 Then cfg.Range("B6").ClearContents',
        'SharedProfile = TextValue(cfg.Range("B8").Value2) = "1" And Len(Trim$(TextValue(cfg.Range("B5").Value2))) > 0 _',
        'If owner <> TextValue(ThisWorkbook.Worksheets("_sync").Range("B7").Value2) Then Err.Raise IDENTITY_ERROR',
        'DataSheetName = CStr(ThisWorkbook.Worksheets("_sync").Range("B3").Value2)',
    ):
        assert line in VBA, line
    cells = sync_cells(configure())
    assert cells["B1"] == WORKBOOK and re.fullmatch(  # StartSync's own test of the id
        "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}", cells["B1"])
    assert cells["B5"] == SERVER and cells["B7"] == str(OWNER) and cells["B8"] == "1"
    assert cells["B6"] is None and cells["B9"] is None
    assert cells["B3"] in openpyxl.load_workbook(io.BytesIO(TEMPLATE), keep_vba=True).sheetnames


def test_only_the_sync_sheet_part_differs_and_every_other_entry_is_the_same_bytes():
    result = configure()
    name = sync_part(TEMPLATE)
    before, after = parts(TEMPLATE), parts(result)
    assert list(before) == list(after)
    assert [part for part in before if before[part] != after[part]] == [name]
    # Not merely equal once unpacked: header and packed bytes are the template's.
    raw_before, raw_after = raw_entries(TEMPLATE), raw_entries(result)
    assert all(raw_before[part] == raw_after[part] for part in before if part != name)
    assert raw_before[name] != raw_after[name]
    assert entry_facts(TEMPLATE) == entry_facts(result)
    assert hashlib.sha256(after["xl/vbaProject.bin"]).hexdigest() == hashlib.sha256(before["xl/vbaProject.bin"]).hexdigest()
    assert after["xl/vbaProject.bin"].startswith(bytes.fromhex("d0cf11e0a1b11ae1"))
    with zipfile.ZipFile(io.BytesIO(result)) as archive:
        assert archive.testzip() is None and archive.comment == b""


def test_the_sheet_is_the_template_sheet_with_three_cells_filled_and_nothing_added():
    name = sync_part(TEMPLATE)
    expected = parts(TEMPLATE)[name].decode("utf-8")
    for ref, value in (("B1", WORKBOOK), ("B5", SERVER), ("B7", str(OWNER))):
        assert expected.count(EMPTY.format(ref=ref)) == 1
        expected = expected.replace(EMPTY.format(ref=ref), TEXT.format(ref=ref, value=value))
    # Byte for byte: so no name, password, key or anything else can have come along.
    assert parts(configure())[name].decode("utf-8") == expected
    assert EMPTY.format(ref="B6") in expected and EMPTY.format(ref="B9") in expected


def test_the_function_has_no_way_to_receive_a_secret_and_is_repeatable():
    assert list(inspect.signature(builder.configure_workbook).parameters) == [
        "template", "workbook_id", "server_url", "owner_id"]
    assert configure() == configure()
    other = configure(workbook_id="6f1c0f6e-4a55-4a0e-9a54-2d2a4d1f7a10", owner_id=7)
    assert sync_cells(other)["B1"] == "6f1c0f6e-4a55-4a0e-9a54-2d2a4d1f7a10" and sync_cells(other)["B7"] == "7"


def test_text_is_escaped_as_xml():
    sheet = parts(TEMPLATE)[sync_part(TEMPLATE)]
    cell = next(cell for cell in builder._scan_cells("sheet", sheet).values() if cell.ref == "B1")
    written = builder._cell_xml(sheet, cell, "B1", "200", 'a&b<c>"d"').decode("utf-8")
    assert "a&amp;b&lt;c&gt;" in written
    assert ET.fromstring(written.replace("<s:c ", f'<s:c xmlns:s="{MAIN}" ')).find(f".//{{{MAIN}}}t").text == 'a&b<c>"d"'


@pytest.mark.parametrize("arguments", [
    {"workbook_id": WORKBOOK.upper()}, {"workbook_id": "{" + WORKBOOK + "}"}, {"workbook_id": WORKBOOK.replace("-", "")},
    {"workbook_id": "not-a-uuid"}, {"workbook_id": ""}, {"workbook_id": None},
    {"server_url": "https://Sync.example.test:8443"}, {"server_url": SERVER + "/"}, {"server_url": "http://sync.example.test"},
    {"server_url": "https://user:secret@sync.example.test"}, {"server_url": ""},
    {"owner_id": 0}, {"owner_id": -1}, {"owner_id": True}, {"owner_id": "42"}, {"owner_id": 4.0},
])
def test_only_checked_values_are_written(arguments):
    with pytest.raises(ValueError):
        configure(**arguments)


# --- the address ---------------------------------------------------------------

def vba_accepts(url: str) -> bool:
    """ValidServerUrl as written in ExcelSync.bas (VBScript: ASCII letters, "$" is the very end)."""
    body = VBA.split("Private Function ValidServerUrl", 1)[1].split("End Function", 1)[0]
    assert "pattern.IgnoreCase = True" in body
    pattern = re.search(r'pattern\.Pattern = "(.*)"', body).group(1)
    return re.fullmatch(pattern, url, re.IGNORECASE | re.ASCII) is not None


ACCEPTED = [
    ("https://example.com", "https://example.com"),
    ("https://example.com/", "https://example.com"),
    ("HTTPS://Example.COM", "https://example.com"),
    ("https://example.com:443", "https://example.com"),
    ("https://example.com:8443/", "https://example.com:8443"),
    ("https://example.com:08443", "https://example.com:8443"),
    ("https://203.0.113.7.nip.io", "https://203.0.113.7.nip.io"),
    ("https://203.0.113.7:9443", "https://203.0.113.7:9443"),
    ("https://xn--e1afmkfd.xn--p1ai", "https://xn--e1afmkfd.xn--p1ai"),
    ("https://intranet", "https://intranet"),
    ("https://localhost", "https://localhost"),
    ("http://127.0.0.1:8000", "http://127.0.0.1:8000"),
    ("http://127.0.0.1:80", "http://127.0.0.1"),
    ("http://localhost:8765/", "http://localhost:8765"),
    ("http://LOCALHOST", "http://localhost"),
]
REFUSED_AS_IN_VBA = [
    # another scheme, or http to anything but this computer
    "http://example.com", "http://example.com:8000", "http://127.0.0.2", "http://127.0.0.1.example.com",
    "http://localhost.example.com", "http://0.0.0.0:8000", "http://[::1]:8000", "https://[::1]", "ftp://example.com",
    "ws://localhost", "file:///c:/book", "javascript:alert(1)", "//example.com", "example.com", "https:/example.com",
    "https://", "https:///", "",
    # a user name or password
    "https://user:secret@example.com", "https://user@example.com", "https://example.com@evil.example",
    "https://example.com:secret@evil.example",
    # anything after the host
    "https://example.com/api", "https://example.com//", "https://example.com/?a=1", "https://example.com?a=1",
    "https://example.com#top", "https://example.com/#top", "https://example.com\\@evil.example", "https:\\\\example.com",
    "https://example.com%2f", "https://exam%70le.com",
    # spaces and control characters
    " https://example.com", "https://example.com ", "https://exa mple.com", "https://example.com\n",
    "https://example.com\r\n", "https://example.com\t", "https://example.com\x00", "https://example.com\n/",
    "https://example.com/\n",
    # hosts that only look ASCII, or are not host names
    "https://пример.рф", "https://\u212aexample.com", "http\u017f://example.com", "https://ｅxample.com",
    "https://example\u3002com", "https://my_host", "https://example.com:", "https://example.com:8a",
    "https://example.com:-1", "https://example.com:+80", "https://example.com:80:80",
]
REFUSED_THOUGH_THE_PATTERN_ALLOWS = [
    # The VBA pattern lets these through, but no server can be reached by them.
    "https://example.com:0", "https://example.com:65536", "https://example.com:99999999", "https://example.com:000443",
    "https://-example.com", "https://example-.com", "https://a..b", "https://.example.com", "https://example.com.",
    "https://.", "https://-", "https://" + "a" * 64 + ".com", "https://" + ".".join(["a" * 60] * 5),
]


def test_the_python_rule_is_the_rule_in_the_vba():
    body = VBA.split("Private Function ValidServerUrl", 1)[1].split("End Function", 1)[0]
    assert re.search(r'pattern\.Pattern = "(.*)"', body).group(1) == builder.VBA_SERVER_URL_PATTERN
    # The same function guards every request the workbook makes.
    assert VBA.count("If Not ValidServerUrl(url) Then") >= 2


@pytest.mark.parametrize("given,written", ACCEPTED)
def test_an_accepted_address_is_written_in_the_one_form_the_workbook_accepts(given, written):
    assert builder.normalize_server_url(given) == written
    assert builder.normalize_server_url(written) == written
    assert vba_accepts(written)
    assert sync_cells(configure(server_url=written))["B5"] == written


@pytest.mark.parametrize("given", REFUSED_AS_IN_VBA)
def test_an_address_the_workbook_would_refuse_is_refused(given):
    assert not vba_accepts(given)
    with pytest.raises(builder.InvalidServerUrl) as refusal:
        builder.normalize_server_url(given)
    # The reason is shown to people and logged: it must not repeat credentials.
    assert "secret" not in str(refusal.value) and "evil" not in str(refusal.value)


@pytest.mark.parametrize("given", REFUSED_THOUGH_THE_PATTERN_ALLOWS)
def test_an_address_no_server_can_have_is_refused_although_the_pattern_allows_it(given):
    assert vba_accepts(given)
    with pytest.raises(builder.InvalidServerUrl):
        builder.normalize_server_url(given)


@pytest.mark.parametrize("given", [None, 5, b"https://example.com", ["https://example.com"]])
def test_only_text_is_an_address(given):
    with pytest.raises(builder.InvalidServerUrl):
        builder.normalize_server_url(given)


# --- templates that must not be used --------------------------------------------

def damaged() -> bytes:
    """One byte of the packed _sync sheet flipped."""
    with zipfile.ZipFile(io.BytesIO(TEMPLATE)) as archive:
        info = archive.getinfo(sync_part(TEMPLATE))
    at = info.header_offset + 30 + len(info.filename) + info.compress_size // 2
    return TEMPLATE[:at] + bytes([TEMPLATE[at] ^ 0xFF]) + TEMPLATE[at + 1:]


BROKEN = {
    "empty": lambda: b"",
    "not an archive": lambda: b"PK\x03\x04 this is not a workbook",
    "cut short": lambda: TEMPLATE[:40000],
    "damaged sheet": damaged,
    "bytes in front": lambda: b"MZ stub" + TEMPLATE,
    "bytes behind": lambda: TEMPLATE + b"trailer",
    "no macros": lambda: repack(replace={"xl/vbaProject.bin": None}),
    "empty macros": lambda: repack(replace={"xl/vbaProject.bin": b""}),
    "no workbook part": lambda: repack(replace={"xl/workbook.xml": None}),
    "no _sync sheet": lambda: repack(replace={"xl/workbook.xml": parts(TEMPLATE)["xl/workbook.xml"].replace(
        b'name="_sync"', b'name="_other"')}),
    "sheet part missing": lambda: repack(replace={sync_part(TEMPLATE): None}),
    "two parts with one name": lambda: repack(add={"xl/workbook.xml": parts(TEMPLATE)["xl/workbook.xml"]}),
    "another layout": lambda: with_sheet(lambda xml: xml.replace("<s:t>owner_id</s:t>", "<s:t>owner</s:t>")),
    "rows moved": lambda: with_sheet(lambda xml: xml.replace("<s:t>server_url</s:t>", "<s:t>shared_token</s:t>", 1)),
    "already has a workbook id": lambda: with_sheet(filled("B1", WORKBOOK)),
    "already has a server": lambda: with_sheet(filled("B5", "https://someone.example")),
    "carries a key": lambda: with_sheet(filled("B6", "excel_" + "k" * 43)),
    "already has an owner": lambda: with_sheet(filled("B7", "1")),
    "data sheet is not there": lambda: with_sheet(lambda xml: xml.replace("<s:t>_transactions</s:t>", "<s:t>_gone</s:t>")),
    "formula where a value goes": lambda: with_sheet(lambda xml: xml.replace(
        TEXT.format(ref="B8", value="1"), '<s:c r="B8" s="200" t="str"><s:f>"1"</s:f><s:v>1</s:v></s:c>')),
    "a cell twice": lambda: with_sheet(lambda xml: xml.replace(EMPTY.format(ref="B1"), EMPTY.format(ref="B1") * 2)),
    "document type": lambda: with_sheet(lambda xml: xml.replace(
        "<s:worksheet", '<!DOCTYPE s:worksheet [<!ENTITY x "y">]><s:worksheet', 1)),
    "not well formed": lambda: with_sheet(lambda xml: xml.replace("</s:sheetData>", "")),
    "utf-16 sheet": lambda: repack(replace={sync_part(TEMPLATE): parts(TEMPLATE)[sync_part(TEMPLATE)].decode(
        "utf-8").replace("encoding='utf-8'", "encoding='utf-16'").encode("utf-16")}),
}


@pytest.mark.parametrize("case", list(BROKEN))
def test_a_template_that_is_not_the_expected_one_is_refused(case):
    with pytest.raises(builder.WorkbookTemplateError):
        configure(BROKEN[case]())


def test_a_refusal_says_what_is_wrong_without_being_a_value_error():
    with pytest.raises(builder.WorkbookTemplateError, match="already has a connection key"):
        configure(BROKEN["carries a key"]())
    with pytest.raises(builder.WorkbookTemplateError, match="another layout"):
        configure(BROKEN["another layout"]())
    assert not issubclass(builder.WorkbookTemplateError, ValueError)


# --- templates written by other tools --------------------------------------------

def test_data_descriptors_and_stored_entries_survive():
    for template in (repack(seekable=False), repack(method=zipfile.ZIP_STORED)):
        result = configure(template)
        name = sync_part(template)
        raw_before, raw_after = raw_entries(template), raw_entries(result)
        assert all(raw_before[part] == raw_after[part] for part in raw_before if part != name)
        assert [fact[:3] for fact in entry_facts(result)] == [fact[:3] for fact in entry_facts(template)]
        assert {part: content for part, content in parts(result).items() if part != name} == {
            part: content for part, content in parts(template).items() if part != name}
        assert sync_cells(result)["B1"] == WORKBOOK and sync_cells(result)["B7"] == str(OWNER)
    with zipfile.ZipFile(io.BytesIO(repack(seekable=False))) as archive:
        assert all(info.flag_bits & 0x08 for info in archive.infolist())  # the case is what it claims


def excel_saved_template() -> bytes:
    """The _sync sheet as Excel itself saves one: default namespace, shared
    strings, numbers as numbers, empty cells absent or bare, a status left."""
    strings = [*builder.SYNC_LABELS, "1", "_transactions", None, "CardXabar"]
    shared = "".join(
        "<si><r><t>Disconnected.</t></r><r><t xml:space=\"preserve\"> Edits kept</t></r></si>" if text is None
        else f"<si><t>{text}</t></si>" for text in strings)
    second = {1: "", 2: '<c r="B2" s="1" t="s"><v>10</v></c>', 3: '<c r="B3" s="1" t="s"><v>11</v></c>',
              4: '<c r="B4" s="1"><v>1</v></c>', 5: '<c r="B5" s="1"/>', 6: "", 7: '<c r="B7" s="1"/>',
              8: '<c r="B8" s="1"><v>1</v></c>', 9: '<c r="B9" s="1" t="s"><v>12</v></c>',
              10: '<c r="B10" s="1" t="s"><v>13</v></c>'}
    rows = "".join(f'<row r="{row}" spans="1:2"><c r="A{row}" t="s"><v>{row - 1}</v></c>{second[row]}</row>'
                   for row in range(1, 11))
    sheet = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
             f'<worksheet xmlns="{MAIN}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
             'mc:Ignorable="x14ac" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac">'
             '<sheetPr codeName="Sheet8"/><dimension ref="A1:B10"/>'
             f'<sheetData>{rows}</sheetData>'
             '<sheetProtection password="B22B" sheet="1" objects="1" scenarios="1"/>'
             '<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>')
    relations = parts(TEMPLATE)["xl/_rels/workbook.xml.rels"].decode("utf-8").replace(
        "</ns0:Relationships>",
        f'<ns0:Relationship Id="rId99" Type="{REL}/sharedStrings" Target="sharedStrings.xml" /></ns0:Relationships>')
    return repack(replace={sync_part(TEMPLATE): sheet.encode("utf-8"),
                           "xl/_rels/workbook.xml.rels": relations.encode("utf-8")},
                  add={"xl/sharedStrings.xml": f'<sst xmlns="{MAIN}">{shared}</sst>'.encode("utf-8")})


def test_a_template_saved_by_excel_itself_is_configured_the_same_way():
    template = excel_saved_template()
    name = sync_part(template)
    before = sync_cells(template)
    assert before["B9"] == "Disconnected. Edits kept" and "B1" not in before and "B6" not in before
    result = configure(template)
    assert [part for part, content in parts(template).items() if parts(result)[part] != content] == [name]
    after = sync_cells(result)
    assert [after[f"A{row}"] for row in range(1, 11)] == list(builder.SYNC_LABELS)
    assert (after["B1"], after["B5"], after["B7"], after["B8"]) == (WORKBOOK, SERVER, str(OWNER), "1")
    assert after["B9"] is None and "B6" not in after          # the status is not inherited, no key appears
    assert (after["B2"], after["B3"], after["B4"], after["B10"]) == ("1", "_transactions", "1", "CardXabar")
    old, new = parts(template)[name].decode("utf-8"), parts(result)[name].decode("utf-8")
    # Cells are spelled the way their neighbours are, and nothing around them moved.
    assert f'<c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>{WORKBOOK}</t></is></c></row>' in new
    assert f'<c r="B7" s="1" t="inlineStr"><is><t>{OWNER}</t></is></c>' in new
    assert '<c r="B8" s="1"><v>1</v></c>' in new and '<c r="B9" s="1"/>' in new
    assert old.split("<sheetData>")[0] == new.split("<sheetData>")[0]
    assert old.split("</sheetData>")[1] == new.split("</sheetData>")[1]
    ET.fromstring(new)
