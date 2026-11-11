"""Attach source-only VBA using the LGPL EPPlus 4.5.3.3 document library.

This authoring path needs Windows PowerShell/.NET Framework, but neither Excel
automation nor AccessVBOM. It does not enable macros or change Office settings.
Excel compiles sources on execution; packaging is not a native runtime test.
The pinned build-only NuGet dependency is cached outside the repository. Its
DLL is not shipped to the server or embedded in the generated workbook.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import tempfile
import urllib.request
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZIP_DEFLATED, ZipFile

EPPLUS_VERSION = "4.5.3.3"
EPPLUS_SHA256 = "5432a85fa9d7cd0a8b2ee1f1263515fec0424722f4df7c2b37e35bc1df44b87b"
EPPLUS_URL = f"https://api.nuget.org/v3-flatcontainer/epplus/{EPPLUS_VERSION}/epplus.{EPPLUS_VERSION}.nupkg"
NS_MAIN = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
NS_REL = "http://schemas.openxmlformats.org/package/2006/relationships"
NS_TYPE = "http://schemas.openxmlformats.org/package/2006/content-types"
VBA_REL = "http://schemas.microsoft.com/office/2006/relationships/vbaProject"


def _epplus_dll() -> Path:
    cache = Path(tempfile.gettempdir()) / "tbsparcer-build" / f"epplus-{EPPLUS_VERSION}"
    cache.mkdir(parents=True, exist_ok=True)
    package = cache / "epplus.nupkg"
    if not package.exists():
        with urllib.request.urlopen(EPPLUS_URL, timeout=60) as response:
            data = response.read(16 * 1024 * 1024)
        if hashlib.sha256(data).hexdigest() != EPPLUS_SHA256:
            raise RuntimeError("EPPlus package checksum mismatch")
        temporary = package.with_suffix(".download")
        temporary.write_bytes(data)
        temporary.replace(package)
    if hashlib.sha256(package.read_bytes()).hexdigest() != EPPLUS_SHA256:
        raise RuntimeError(f"EPPlus cache checksum mismatch: {package}")
    # Extract only the verified library and its license, never arbitrary paths.
    with ZipFile(package) as archive:
        for source, target in (("lib/net40/EPPlus.dll", "EPPlus.dll"), ("lgpl-3.0.txt", "LICENSE.txt")):
            (cache / target).write_bytes(archive.read(source))
    return cache / "EPPlus.dll"


def build_project(sources: Path, sheet_codenames: list[str]) -> bytes:
    if os.name != "nt":
        raise RuntimeError("Portable VBA authoring requires Windows .NET Framework (Excel is not required)")
    # EPPlus 4.5's source code page is 1252. UI labels are Unicode worksheet
    # cells; a future non-ASCII source must not silently become '?' in the VBA.
    for filename in ("ThisWorkbook.cls", "Json.bas", "WindowsStore.bas", "ExcelSync.bas", "BankViews.bas"):
        (sources / filename).read_text("utf-8").encode("cp1252", errors="strict")
    dll = _epplus_dll()
    shell = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    if not shell.is_file():
        raise RuntimeError("Windows PowerShell/.NET Framework is unavailable")
    with tempfile.TemporaryDirectory(prefix="tbs-excel-vba-") as temporary:
        folder = Path(temporary)
        config = folder / "build.json"
        output = folder / "project.xlsm"
        config.write_text(json.dumps({"dll": str(dll), "sources": str(sources.resolve()),
                                      "output": str(output), "sheets": sheet_codenames}), encoding="utf-8")
        process = subprocess.run([str(shell), "-NoProfile", "-NonInteractive", "-File",
                                  str(Path(__file__).with_name("package_vba.ps1")), str(config)],
                                 capture_output=True, text=True, timeout=60)
        if process.returncode:
            raise RuntimeError(f"VBA package build failed: {process.stderr.strip() or process.stdout.strip()}")
        with ZipFile(output) as archive:
            return archive.read("xl/vbaProject.bin")


def package_workbook(shell: Path, output: Path, sources: Path) -> dict:
    """Preserve all existing shell parts, adding VBA and explicit code names."""
    if shell.resolve() == output.resolve() or output.suffix.lower() != ".xlsm":
        raise ValueError("Use a separate .xlsm output")
    with ZipFile(shell) as archive:
        parts = {info.filename: archive.read(info) for info in archive.infolist()}
    workbook = ET.fromstring(parts["xl/workbook.xml"])
    properties = workbook.find(f"{{{NS_MAIN}}}workbookPr")
    if properties is None:
        properties = ET.Element(f"{{{NS_MAIN}}}workbookPr")
        workbook.insert(0, properties)
    properties.set("codeName", "ThisWorkbook")
    parts["xl/workbook.xml"] = ET.tostring(workbook, encoding="utf-8", xml_declaration=True)
    codenames = []
    for index, path in enumerate(sorted(p for p in parts if re.fullmatch(r"xl/worksheets/sheet\d+\.xml", p)), 1):
        sheet = ET.fromstring(parts[path])
        properties = sheet.find(f"{{{NS_MAIN}}}sheetPr")
        if properties is None:
            properties = ET.Element(f"{{{NS_MAIN}}}sheetPr")
            sheet.insert(0, properties)
        name = f"Sheet{index}"
        codenames.append(name)
        properties.set("codeName", name)
        parts[path] = ET.tostring(sheet, encoding="utf-8", xml_declaration=True)
    rels = ET.fromstring(parts["xl/_rels/workbook.xml.rels"])
    if any(r.get("Type") == VBA_REL for r in rels):
        raise ValueError("Shell already has a VBA project")
    ids = {r.get("Id") for r in rels}
    next_id = 1
    while f"rId{next_id}" in ids:
        next_id += 1
    ET.SubElement(rels, f"{{{NS_REL}}}Relationship", Id=f"rId{next_id}", Type=VBA_REL, Target="vbaProject.bin")
    parts["xl/_rels/workbook.xml.rels"] = ET.tostring(rels, encoding="utf-8", xml_declaration=True)
    types = ET.fromstring(parts["[Content_Types].xml"])
    for item in types:
        if item.get("PartName") == "/xl/workbook.xml":
            item.set("ContentType", "application/vnd.ms-excel.sheet.macroEnabled.main+xml")
    ET.SubElement(types, f"{{{NS_TYPE}}}Override", PartName="/xl/vbaProject.bin", ContentType="application/vnd.ms-office.vbaProject")
    parts["[Content_Types].xml"] = ET.tostring(types, encoding="utf-8", xml_declaration=True)
    parts["xl/vbaProject.bin"] = build_project(sources, codenames)
    output.parent.mkdir(parents=True, exist_ok=True)
    with ZipFile(output, "w", ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, content)
    # Normalize newlines so a Windows build can be checked on a Linux checkout.
    source_hashes = {name: hashlib.sha256((sources / name).read_text("utf-8").encode("utf-8")).hexdigest()
                     for name in ("ThisWorkbook.cls", "Json.bas", "WindowsStore.bas", "ExcelSync.bas", "BankViews.bas")}
    manifest = {"protocol_version": 1, "source_encoding": "utf-8-lf",
                "artifact_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
                "source_sha256": source_hashes, "native_compiled": False,
                "packager": f"EPPlus {EPPLUS_VERSION} (LGPL-3.0)"}
    output.with_suffix(".manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return {"output": str(output.resolve()), "bytes": output.stat().st_size,
            "modules": len(codenames) + 5, "native_compiled": False,
            "packager": f"EPPlus {EPPLUS_VERSION} (LGPL-3.0)"}
