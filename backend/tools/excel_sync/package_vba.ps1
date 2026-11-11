param([Parameter(Mandatory=$true)][string]$ConfigPath)
$ErrorActionPreference = 'Stop'
$config = Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
Add-Type -Path $config.dll
$package = New-Object OfficeOpenXml.ExcelPackage
try {
    foreach ($name in $config.sheets) {
        $package.Workbook.Worksheets.Add($name) | Out-Null
    }
    $package.Workbook.CreateVBAProject()
    $project = $package.Workbook.VbaProject
    $project.Name = 'TBSExcelSync'
    $workbookSource = [IO.File]::ReadAllText((Join-Path $config.sources 'ThisWorkbook.cls'))
    $bodyStart = $workbookSource.IndexOf('Option Explicit')
    if ($bodyStart -lt 0) { throw 'Workbook event module has no Option Explicit' }
    $project.Modules['ThisWorkbook'].Code = $workbookSource.Substring($bodyStart)
    foreach ($file in @('Json.bas', 'WindowsStore.bas', 'ExcelSync.bas', 'BankViews.bas')) {
        $source = [IO.File]::ReadAllText((Join-Path $config.sources $file))
        $match = [regex]::Match($source, '(?m)^Attribute VB_Name = "([A-Za-z0-9_]+)"\r?$')
        if (-not $match.Success) { throw "Invalid module name in $file" }
        $name = $match.Groups[1].Value
        $module = $project.Modules.AddModule($name)
        $module.Code = [regex]::Replace($source, '(?m)^Attribute VB_Name = .*\r?\n', '')
    }
    # This writes source and host metadata only; it never executes a macro.
    $package.SaveAs([IO.FileInfo]::new($config.output))
} finally {
    $package.Dispose()
}
