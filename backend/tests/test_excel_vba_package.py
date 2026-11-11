"""Distribution checks; these deliberately do not claim to execute VBA."""
import hashlib
import json
import re
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import pytest

BACKEND = Path(__file__).resolve().parents[1]
ARTIFACT = BACKEND / "assets/excel_sync_template.xlsm"
SOURCES = BACKEND / "tools/excel_sync"
MAIN = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"


def test_parameterless_calls_cannot_be_misread_as_vba_line_labels():
    # Native Excel exposed `Persist: ...` as a label, silently skipping the
    # call or failing compilation when the label occurs twice in a procedure.
    for path in [*SOURCES.glob("*.bas"), *SOURCES.glob("*.cls")]:
        source = path.read_text("utf-8")
        procedures = set(re.findall(r"(?im)^\s*(?:(?:Public|Private|Friend)\s+)?(?:Sub|Function)\s+(\w+)", source))
        labels = set(re.findall(r"(?m)^\s*(\w+)\s*:", source))
        assert not (procedures & labels), (path.name, procedures & labels)


def test_shipped_workbook_matches_the_reviewed_sources():
    manifest = json.loads(ARTIFACT.with_suffix(".manifest.json").read_text("utf-8"))
    assert manifest["protocol_version"] == 1
    assert manifest["native_compiled"] is False
    assert manifest["source_encoding"] == "utf-8-lf"
    assert manifest["artifact_sha256"] == hashlib.sha256(ARTIFACT.read_bytes()).hexdigest()
    for filename, expected in manifest["source_sha256"].items():
        assert expected == hashlib.sha256((SOURCES / filename).read_text("utf-8").encode("utf-8")).hexdigest(), filename


def test_macro_project_is_wired_to_the_workbook_and_all_sheets():
    with ZipFile(ARTIFACT) as archive:
        assert archive.testzip() is None
        assert archive.read("xl/vbaProject.bin").startswith(bytes.fromhex("d0cf11e0a1b11ae1"))
        workbook = ET.fromstring(archive.read("xl/workbook.xml"))
        assert workbook.find(f"{{{MAIN}}}workbookPr").get("codeName") == "ThisWorkbook"
        names = []
        for filename in archive.namelist():
            if re.fullmatch(r"xl/worksheets/sheet\d+\.xml", filename):
                names.append(ET.fromstring(archive.read(filename)).find(f"{{{MAIN}}}sheetPr").get("codeName"))
        assert len(names) == len(set(names)) == 10
        assert set(names) == {f"Sheet{i}" for i in range(1, 11)}
        sheets = workbook.find(f"{{{MAIN}}}sheets")
        assert [s.get("name") for s in sheets if s.get("state", "visible") == "visible"] == ["Доступ"]
        # Receipts are veryHidden on disk until this computer is activated.
        assert {s.get("name"): s.get("state") for s in sheets if s.get("name") in {"CardXabar", "HUMO Card", "Все транзакции"}} == {
            "CardXabar": "veryHidden", "HUMO Card": "veryHidden", "Все транзакции": "veryHidden"}
        rels = ET.fromstring(archive.read("xl/_rels/workbook.xml.rels"))
        vba = [r for r in rels if r.get("Type", "").endswith("/vbaProject")]
        assert len(vba) == 1 and vba[0].get("Target") == "vbaProject.bin"
        content_types = ET.fromstring(archive.read("[Content_Types].xml"))
        workbook_type = next(t for t in content_types if t.get("PartName") == "/xl/workbook.xml")
        assert workbook_type.get("ContentType") == "application/vnd.ms-excel.sheet.macroEnabled.main+xml"
        assert "xl/connections.xml" not in archive.namelist()
        assert not any("queryTable" in name for name in archive.namelist())


def test_vba_container_extracts_exact_sources_with_an_independent_reader():
    # Optional build-time tool, never a backend production dependency.
    olevba = pytest.importorskip("oletools.olevba", reason="Install oletools to independently inspect the VBA container")
    parser = olevba.VBA_Parser(str(ARTIFACT))
    try:
        macros = {filename: code for _, _, filename, code in parser.extract_macros()}
    finally:
        parser.close()
    assert set(macros) == {"ThisWorkbook.cls", "SyncJson.bas", "SyncWindows.bas", "ExcelSync.bas", "BankViews.bas",
                           *(f"Sheet{i}.cls" for i in range(1, 11))}
    files = {"ThisWorkbook.cls": "ThisWorkbook.cls", "SyncJson.bas": "Json.bas",
             "SyncWindows.bas": "WindowsStore.bas", "ExcelSync.bas": "ExcelSync.bas", "BankViews.bas": "BankViews.bas"}
    def body(value):
        return value[value.index("Option Explicit"):].replace("\r", "").strip()
    for macro, source in files.items():
        assert body(macros[macro]) == body((SOURCES / source).read_text("utf-8")), macro
