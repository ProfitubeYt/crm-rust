param([Parameter(Mandatory=$true)][string]$Settings)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$cfg=Get-Content -LiteralPath $Settings -Raw -Encoding UTF8 | ConvertFrom-Json
$excel=$null; $book=$null
try {
    # prepare_shell owns both bank layouts. Native authoring only adds VBA.
    $excel=New-Object -ComObject Excel.Application
    $excel.Visible=$false
    $excel.DisplayAlerts=$false
    $excel.AskToUpdateLinks=$false
    $excel.EnableEvents=$false
    $excel.AutomationSecurity=3
    $book=$excel.Workbooks.Open([string]$cfg.working,0,$false)
    $project=$null
    try {$project=$book.VBProject} catch {$project=$null}
    $vbom=$null -ne $project
    if($cfg.probe) {
        @{excel_version=$excel.Version;access_vbom=$vbom} | ConvertTo-Json | Set-Content -LiteralPath $cfg.result -Encoding UTF8
        return
    }
    if(-not $cfg.prepare_only -and -not $vbom) {
        throw 'Excel blocks programmatic VBA access. No security setting was changed. Use portable source packaging or a machine already authorized for VBA access.'
    }
    if(-not $cfg.prepare_only) {
        foreach($file in @('Json.bas','WindowsStore.bas','ExcelSync.bas','BankViews.bas')) {
            $text=Get-Content -LiteralPath (Join-Path $cfg.sources $file) -Raw -Encoding UTF8
            $name=[regex]::Match($text,'Attribute VB_Name = "([^"]+)"').Groups[1].Value
            $text=[regex]::Replace($text,'(?m)^Attribute[^\r\n]*\r?\n','')
            $component=$project.VBComponents.Add(1)
            $component.Name=$name
            $component.CodeModule.AddFromString($text)
        }
        $text=Get-Content -LiteralPath (Join-Path $cfg.sources 'ThisWorkbook.cls') -Raw -Encoding UTF8
        $text=$text.Substring($text.IndexOf('Option Explicit'))
        $project.VBComponents.Item($book.CodeName).CodeModule.AddFromString($text)
        $format=52
    } else {$format=51}
    $excel.CalculateFull()
    $book.SaveAs([string]$cfg.out,$format)
    @{path=$cfg.out;excel_version=$excel.Version;access_vbom=$vbom;macro_client=(-not $cfg.prepare_only);
      workbook_id_blank=($null -eq $book.Worksheets.Item('_sync').Range('B1').Value2);
      queries=$book.Queries.Count;connections=$book.Connections.Count;
      visible_sheets=@('CardXabar','HUMO Card');credential_in_workbook=$false} | ConvertTo-Json | Set-Content -LiteralPath $cfg.result -Encoding UTF8
} finally {
    if($null -ne $book){$book.Close($false)}
    if($null -ne $excel){$excel.Quit();[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)}
}