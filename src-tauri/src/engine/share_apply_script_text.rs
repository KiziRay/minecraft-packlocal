//! S1：分享包收件端 PowerShell 腳本與說明的文字（由 share_apply_script 組裝、填入檔名常數）。
//!
//! `__BACKUP_DIR__` 等佔位字由 `share_apply_script::fill_names` 換成常數；`__PACK_ZIP__` 為主翻譯資源包檔名。

pub(super) const APPLY_PARAMS: &str = r##"param(
  [Parameter(Position = 0)][string]$PackDir,
  [string]$GameDir
)
"##;

pub(super) const RESTORE_PARAMS: &str = r##"param(
  [string]$BackupDir
)
"##;

/// 兩支腳本共用：編碼、輸出、確認、雜湊、遊戲是否在執行。
pub(super) const COMMON: &str = r##"$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch { }
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding $false
$script:Utf8Bom = New-Object System.Text.UTF8Encoding $true

function Say([string]$text) { Write-Host $text }
function Warn([string]$text) { Write-Host $text -ForegroundColor Yellow }
function Bad([string]$text) { Write-Host $text -ForegroundColor Red }

function Wait-Close {
  # 由「還原這次套用.cmd」啟動時，.cmd 結尾的 pause 負責停住（PowerShell 本身起不來也看得到）。
  if ($env:MCPL_FROM_CMD -eq '1') { return }
  Write-Host ''
  Write-Host '按 Enter 關閉這個視窗。'
  try { [void][Console]::In.ReadLine() } catch { }
}

function Stop-Here([string]$why, [int]$code) {
  Write-Host ''
  Bad $why
  Wait-Close
  exit $code
}

# 預設不繼續：只有輸入 y（或 yes）才回 true。
function Ask-Yes([string]$question) {
  Write-Host ''
  Warn $question
  Write-Host '要繼續請輸入 y 再按 Enter；直接按 Enter 或輸入其他內容就取消。'
  $answer = $null
  try { $answer = [Console]::In.ReadLine() } catch { $answer = $null }
  if ($null -eq $answer) { return $false }
  $a = $answer.Trim().ToLowerInvariant()
  return ($a -eq 'y' -or $a -eq 'yes')
}

function Hash-File([string]$path) {
  return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Parent-Of([string]$path) {
  $i = $path.LastIndexOf('\')
  if ($i -lt 0) { return '' }
  return $path.Substring(0, $i)
}

$script:TopDirs = @(__TOP_DIRS__)

# 只接受：分享內容的頂層資料夾底下的檔，或頂層的雲端捷徑檔。
# Windows 會忽略路徑段結尾的點與空白（「.mcpl-share-backup.」＝「.mcpl-share-backup」），這種段一律拒絕。
function Test-SafeRel([string]$rel, [switch]$Dir) {
  if (-not $rel) { return $false }
  if ($rel.Contains(':') -or $rel.StartsWith('\') -or $rel.Contains('/')) { return $false }
  $parts = $rel.Split('\')
  foreach ($part in $parts) {
    if ($part -eq '' -or $part.EndsWith('.') -or $part.EndsWith(' ') -or $part -eq '__BACKUP_DIR__') { return $false }
  }
  if ($parts.Count -eq 1) {
    if ($Dir) { return ($script:TopDirs -contains $rel) }
    return ($rel -eq '__SHORTCUT__')
  }
  return ($script:TopDirs -contains $parts[0])
}

function Test-IsGameDir([string]$dir) {
  return (Test-Path -LiteralPath (Join-Path $dir 'mods') -PathType Container)
}

# 回傳 'yes'、'no'、'unknown-query'（查不到行程清單）或 'unknown-cmdline'（有 Java 行程但讀不到命令列）。
# 判斷不了就停止，不猜。
function Test-GameRunning([string]$gameDir) {
  # 測試用，只能把結果改成「停止」，不能改成放行。
  $forced = $env:MCPL_SHARE_TEST_GAME_RUNNING
  if ($forced -eq 'yes') { return 'yes' }
  if ($forced -eq 'unknown') { return 'unknown-cmdline' }
  try {
    $procs = @(Get-CimInstance -ClassName Win32_Process -Filter "Name='javaw.exe' or Name='java.exe'" -ErrorAction Stop)
  } catch {
    return 'unknown-query'
  }
  $lines = @()
  foreach ($p in $procs) {
    $cl = [string]$p.CommandLine
    if (-not $cl) { return 'unknown-cmdline' }
    $lines += $cl
  }
  # 測試用：額外加一條假的 Java 命令列（只會多擋，不會放行）。
  if ($env:MCPL_SHARE_TEST_EXTRA_CMDLINE) { $lines += $env:MCPL_SHARE_TEST_EXTRA_CMDLINE }
  $needles = @($gameDir.Replace('/', '\').TrimEnd('\').ToLowerInvariant())
  $leafOf = { param($x) $x.Substring($x.LastIndexOf('\') + 1) }
  # Prism／MultiMC：遊戲資料夾是實例裡的 .minecraft／minecraft，命令列常只出現實例資料夾（例如 natives）。
  if (@('.minecraft', 'minecraft') -contains (& $leafOf $needles[0])) { $needles += (Parent-Of $needles[0]) }
  foreach ($cl in $lines) {
    $hay = $cl.Replace('/', '\').ToLowerInvariant()
    foreach ($needle in $needles) {
      if (-not $needle) { continue }
      # 後面要接分隔符、引號、空白或結尾，避免「atm10」誤中「atm10-2」
      if ($hay.EndsWith($needle) -or $hay.Contains($needle + '\') -or $hay.Contains($needle + '"') -or $hay.Contains($needle + ' ')) { return 'yes' }
      $leaf = & $leafOf $needle
      if ($leaf.Length -ge 4 -and $leaf -ne '.minecraft' -and $leaf -ne 'minecraft' -and $hay.Contains('\' + $leaf + '\')) { return 'yes' }
    }
  }
  return 'no'
}

function Stop-IfGameRunning([string]$gameDir, [string]$verb) {
  $running = Test-GameRunning $gameDir
  if ($running -eq 'yes') {
    Stop-Here "偵測到有 Minecraft／Java 程式在執行，請先關閉再$verb。沒有改動任何檔案。" 1
  }
  if ($running -eq 'unknown-cmdline') {
    Stop-Here ("無法確認遊戲有沒有在執行：有 Java 程式在執行，但讀不到它的啟動資訊，" +
      "常見原因是它以系統管理員身分執行。`n做法：關閉 Minecraft、啟動器和其他 Java 程式後再$verb；" +
      "若仍出現，對這個檔案按右鍵選「以系統管理員身分執行」。沒有改動任何檔案。") 1
  }
  if ($running -ne 'no') {
    Stop-Here "無法確認遊戲有沒有在執行（查不到正在執行的程式清單）。請關閉 Minecraft 和啟動器後再$verb。沒有改動任何檔案。" 1
  }
}

function Read-Rows([string]$path) {
  $rows = @()
  foreach ($line in (Get-Content -LiteralPath $path -Encoding UTF8)) {
    if (-not $line -or $line.StartsWith('#')) { continue }
    $rows += ,($line.Split("`t"))
  }
  return ,$rows
}

function Show-Names([string]$title, $names) {
  $list = @($names | Sort-Object)
  if ($list.Count -eq 0) { return }
  Say "$title（$($list.Count) 個）："
  $shown = 0
  foreach ($n in $list) {
    if ($shown -ge 20) { Say "  …還有 $($list.Count - 20) 個"; break }
    Say "  $n"
    $shown++
  }
}

"##;

pub(super) const APPLY_BODY: &str = r##"$packZip = '__PACK_ZIP__'

try {
  Say '=== 模組包繁中翻譯：套用 ==='

  # 1. 分享包本身
  if (-not $PackDir) { $PackDir = $PSScriptRoot }
  if (-not (Test-Path -LiteralPath $PackDir -PathType Container)) { Stop-Here "找不到解壓後的翻譯檔資料夾：$PackDir。沒有改動任何檔案。" 2 }
  $PackDir = (Get-Item -LiteralPath $PackDir).FullName.TrimEnd('\')
  $listPath = Join-Path $PackDir '__FILES_LIST__'
  $restoreSrc = Join-Path $PackDir '__RESTORE_PS1__'
  if (-not (Test-Path -LiteralPath $listPath -PathType Leaf) -or -not (Test-Path -LiteralPath $restoreSrc -PathType Leaf)) {
    Stop-Here '分享檔少了必要的檔案，可能下載不完整。請重新下載分享檔。沒有改動任何檔案。' 2
  }

  # 2. 遊戲資料夾
  if (-not $GameDir) {
    Add-Type -AssemblyName System.Windows.Forms
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.Description = '請選擇 Minecraft 遊戲資料夾（裡面有 mods 資料夾的那一層）。'
    $dialog.ShowNewFolderButton = $false
    if ($dialog.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) { Stop-Here '已取消，沒有改動任何檔案。' 1 }
    $GameDir = $dialog.SelectedPath
  }
  if (-not (Test-Path -LiteralPath $GameDir -PathType Container)) { Stop-Here "找不到這個資料夾：$GameDir`n沒有改動任何檔案。" 1 }
  $mc = (Get-Item -LiteralPath $GameDir).FullName.TrimEnd('\')
  Say "遊戲資料夾：$mc"
  if (-not (Test-IsGameDir $mc)) {
    $hint = ''
    foreach ($sub in @('.minecraft', 'minecraft')) {
      $inner = Join-Path $mc $sub
      if (Test-IsGameDir $inner) { $hint = "`n看起來真正的遊戲資料夾是裡面的：`n  $inner`n請重新執行，改選那一層。" }
    }
    Stop-Here "這個資料夾裡沒有 mods 資料夾，不像 Minecraft 遊戲資料夾。$hint`n沒有改動任何檔案。" 1
  }
  $hasOptions = Test-Path -LiteralPath (Join-Path $mc 'options.txt') -PathType Leaf
  $hasConfig = Test-Path -LiteralPath (Join-Path $mc 'config') -PathType Container
  if (-not $hasOptions -and -not $hasConfig) {
    Stop-Here '這個資料夾有 mods，但沒有 options.txt 也沒有 config，無法確認是遊戲資料夾。請先用啟動器開一次遊戲再關掉，然後重新執行。沒有改動任何檔案。' 1
  }

  # 3. 遊戲不能開著
  Stop-IfGameRunning $mc '套用'

  # 4. 分享包完整
  $entries = @()
  foreach ($row in (Read-Rows $listPath)) {
    if ($row.Count -ne 2 -or -not (Test-SafeRel $row[1])) { Stop-Here "分享檔的檔案清單有不正確的內容，為了安全不套用。沒有改動任何檔案。" 2 }
    $rel = $row[1]
    $entries += [pscustomobject]@{ Rel = $rel; Hash = $row[0].ToLowerInvariant(); Src = (Join-Path $PackDir $rel); Dst = (Join-Path $mc $rel); Old = '' }
  }
  if ($entries.Count -eq 0) { Stop-Here '分享檔裡沒有要套用的檔案。沒有改動任何檔案。' 2 }
  foreach ($e in $entries) {
    if (-not (Test-Path -LiteralPath $e.Src -PathType Leaf) -or (Hash-File $e.Src) -ne $e.Hash) {
      Stop-Here "分享檔不完整或已損壞（$($e.Rel)）。請重新下載分享檔。沒有改動任何檔案。" 2
    }
  }

  # 5. 模組比對（不同就列出，預設取消）
  $modsListPath = Join-Path $PackDir '__MODS_LIST__'
  $mine = @{}
  foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $mc 'mods') -File -Force)) {
    if ($f.Name.ToLowerInvariant().EndsWith('.jar')) { $mine[$f.Name.ToLowerInvariant()] = [pscustomobject]@{ Name = $f.Name; Size = [int64]$f.Length } }
  }
  if (-not (Test-Path -LiteralPath $modsListPath -PathType Leaf)) {
    Warn '分享檔沒有附分享者的模組清單，無法比對你的模組和分享者的是否相同。'
    if (-not (Ask-Yes '模組不同時翻譯可能對不上，任務與設定檔也可能換成分享者的版本（會先備份，可以還原）。仍要套用嗎？')) { Stop-Here '已取消，沒有改動任何檔案。' 1 }
  } else {
    $theirs = @{}
    foreach ($row in (Read-Rows $modsListPath)) {
      if ($row.Count -eq 2) { $theirs[$row[1].ToLowerInvariant()] = [pscustomobject]@{ Name = $row[1]; Size = [int64]$row[0] } }
    }
    $onlyTheirs = @(); $onlyMine = @(); $sizeDiff = @()
    foreach ($k in @($theirs.Keys)) {
      if (-not $mine.ContainsKey($k)) { $onlyTheirs += $theirs[$k].Name }
      elseif ($mine[$k].Size -ne $theirs[$k].Size) { $sizeDiff += $theirs[$k].Name }
    }
    foreach ($k in @($mine.Keys)) { if (-not $theirs.ContainsKey($k)) { $onlyMine += $mine[$k].Name } }
    if (($onlyTheirs.Count + $onlyMine.Count + $sizeDiff.Count) -gt 0) {
      Say ''
      Warn '你的模組和分享者的不一樣：'
      Show-Names '分享者有、你沒有' $onlyTheirs
      Show-Names '你有、分享者沒有' $onlyMine
      Show-Names '同名但檔案大小不同（版本可能不同）' $sizeDiff
      if (-not (Ask-Yes '模組不同時翻譯可能對不上，任務與設定檔也可能換成分享者的版本（會先備份，可以還原）。仍要套用嗎？')) { Stop-Here '已取消，沒有改動任何檔案。' 1 }
    } else {
      Say '模組和分享者相同。'
    }
  }

  # 6. 盤點：會覆蓋哪些、會新增哪些、會新建哪些資料夾
  $overwrite = @(); $add = @(); $newDirs = @(); $seenDirs = @{}
  foreach ($e in $entries) {
    if (Test-Path -LiteralPath $e.Dst -PathType Container) { Stop-Here "遊戲資料夾裡的 $($e.Rel) 是資料夾，不是檔案，無法套用。沒有改動任何檔案。" 2 }
    if (Test-Path -LiteralPath $e.Dst -PathType Leaf) {
      try { $e.Old = Hash-File $e.Dst } catch {
        Stop-Here ("讀不到遊戲資料夾裡的 $($e.Rel)，無法先備份。`n原因：" + $_.Exception.Message + "`n這個檔可能正被其他程式使用（例如遊戲、啟動器或編輯器）。關閉後再重新執行。沒有改動任何檔案。") 2
      }
      $overwrite += $e
    } else {
      $add += $e
      $parent = Parent-Of $e.Rel
      while ($parent) {
        if (Test-Path -LiteralPath (Join-Path $mc $parent)) { break }
        $key = $parent.ToLowerInvariant()
        if (-not $seenDirs.ContainsKey($key)) { $seenDirs[$key] = $true; $newDirs += $parent }
        $parent = Parent-Of $parent
      }
    }
  }

  # 輸入 y 可能隔了一段時間：備份前再確認一次遊戲沒開
  Stop-IfGameRunning $mc '套用'

  # 7. 備份（失敗就中止，不覆蓋任何檔）
  $bakRoot = Join-Path $mc '__BACKUP_DIR__'
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $bak = Join-Path $bakRoot $stamp
  $n = 2
  while (Test-Path -LiteralPath $bak) { $bak = Join-Path $bakRoot "$stamp-$n"; $n++ }
  Say ''
  Say "套用前先備份會被覆蓋的 $($overwrite.Count) 個檔到：`n  $bak"
  try {
    [void][IO.Directory]::CreateDirectory($bak)
    $filesRoot = Join-Path $bak 'files'
    foreach ($e in $overwrite) {
      $to = Join-Path $filesRoot $e.Rel
      [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($to))
      [IO.File]::Copy($e.Dst, $to, $false)
      if ((Hash-File $to) -ne $e.Old) { throw "備份出來的內容和原檔不同：$($e.Rel)" }
    }
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add('# mcpl-share-backup v1')
    foreach ($e in $overwrite) { $lines.Add("overwritten`t$($e.Old)`t$($e.Hash)`t$($e.Rel)") }
    foreach ($e in $add) { $lines.Add("added`t-`t$($e.Hash)`t$($e.Rel)") }
    foreach ($d in $newDirs) { $lines.Add("newdir`t-`t-`t$d") }
    [IO.File]::WriteAllLines((Join-Path $bak 'manifest.tsv'), [string[]]$lines.ToArray(), $script:Utf8NoBom)
    $human = New-Object System.Collections.Generic.List[string]
    $human.Add('這次套用分享翻譯的紀錄')
    $human.Add("時間：$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
    $human.Add("遊戲資料夾：$mc")
    $human.Add('')
    $human.Add("覆蓋的檔（原檔已備份在本資料夾的 files 裡）：$($overwrite.Count) 個")
    foreach ($e in $overwrite) { $human.Add("  $($e.Rel)") }
    $human.Add('')
    $human.Add("新增的檔（還原時會刪除）：$($add.Count) 個")
    foreach ($e in $add) { $human.Add("  $($e.Rel)") }
    $human.Add('')
    $human.Add('想還原：按兩下本資料夾的「__RESTORE_CMD__」。')
    [IO.File]::WriteAllLines((Join-Path $bak '清單.txt'), [string[]]$human.ToArray(), $script:Utf8Bom)
    [IO.File]::Copy($restoreSrc, (Join-Path $bak '__RESTORE_PS1__'), $false)
    $launcher = "@echo off`r`nset MCPL_FROM_CMD=1`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"%~dp0__RESTORE_PS1__`"`r`npause`r`n"
    [IO.File]::WriteAllText((Join-Path $bak '__RESTORE_CMD__'), $launcher, [System.Text.Encoding]::ASCII)
  } catch {
    Stop-Here ("備份失敗，所以沒有覆蓋任何檔案。`n原因：" + $_.Exception.Message + "`n（備份資料夾可以刪除：$bak）") 2
  }

  # 8. 套用：每個檔先寫到同資料夾的暫存檔、驗過雜湊，再取代／改名成目的檔。
  #    目的檔因此只會是舊版或新版，不會是寫到一半的半截檔。
  $done = 0
  foreach ($e in $entries) {
    $tmp = $e.Dst + '.mcpl-tmp-' + $PID
    try {
      [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($e.Dst))
      [IO.File]::Copy($e.Src, $tmp, $true)
      if ((Hash-File $tmp) -ne $e.Hash) { throw '暫存檔寫入後內容不對。' }
      if ($e.Old) { [IO.File]::Replace($tmp, $e.Dst, [NullString]::Value) } else { [IO.File]::Move($tmp, $e.Dst) }
      if ((Hash-File $e.Dst) -ne $e.Hash) { throw '寫入後內容不對。' }
      $done++
    } catch {
      $why = $_.Exception.Message
      try { if ([IO.File]::Exists($tmp)) { [IO.File]::Delete($tmp) } } catch { }
      # 把這一個檔放回套用前的狀態：覆蓋的檔從備份放回，新增的檔刪掉
      $state = '已維持原本的內容。'
      try {
        if ($e.Old) {
          if (-not [IO.File]::Exists($e.Dst) -or (Hash-File $e.Dst) -ne $e.Old) {
            [IO.File]::Copy((Join-Path (Join-Path $bak 'files') $e.Rel), $e.Dst, $true)
            if ((Hash-File $e.Dst) -ne $e.Old) { throw '放回後內容不對。' }
          }
        } elseif ([IO.File]::Exists($e.Dst)) {
          [IO.File]::Delete($e.Dst)
        }
      } catch {
        $state = "無法自動放回（" + $_.Exception.Message + "），請用下面的還原處理。"
      }
      Stop-Here ("套用到一半停止（已完成 $done / $($entries.Count) 個檔）。`n失敗的檔：$($e.Rel)，$state`n原因：" + $why + "`n這個檔可能正被其他程式使用（例如遊戲、啟動器或編輯器）。`n前面已套用的 $done 個檔可以還原：按兩下`n  $bak\__RESTORE_CMD__") 2
    }
  }

  Say ''
  Say '完成：翻譯已套用到'
  Say "  $mc"
  Say "覆蓋了 $($overwrite.Count) 個既有檔（套用前已備份），新增了 $($add.Count) 個檔。"
  Say "備份在：`n  $bak"
  Say '想還原：到上面的備份資料夾，按兩下「__RESTORE_CMD__」。'
  Say '接著：開啟遊戲 → 選項 → 語言，選「繁體中文（台灣）」。'
  if (@($entries | Where-Object { $_.Rel -eq "resourcepacks\$packZip" }).Count -gt 0) {
    Say "資源包：在資源包列表啟用「$packZip」。"
  }
  Wait-Close
  exit 0
} catch {
  Stop-Here ("發生未預期的錯誤，已停止。`n原因：" + $_.Exception.Message) 2
}
"##;

pub(super) const RESTORE_BODY: &str = r##"try {
  Say '=== 模組包繁中翻譯：還原這次套用 ==='
  if (-not $BackupDir) { $BackupDir = $PSScriptRoot }
  if (-not (Test-Path -LiteralPath $BackupDir -PathType Container)) { Stop-Here "找不到備份資料夾：$BackupDir。沒有改動任何檔案。" 2 }
  $BackupDir = (Get-Item -LiteralPath $BackupDir).FullName.TrimEnd('\')
  $manifest = Join-Path $BackupDir 'manifest.tsv'
  if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { Stop-Here '備份資料夾裡找不到清單（manifest.tsv），無法還原。沒有改動任何檔案。' 2 }
  $first = @(Get-Content -LiteralPath $manifest -Encoding UTF8 -TotalCount 1)
  if ($first.Count -eq 0 -or $first[0] -ne '# mcpl-share-backup v1') { Stop-Here '備份清單格式不認得，無法還原。沒有改動任何檔案。' 2 }
  $bakParent = [IO.Path]::GetDirectoryName($BackupDir)
  if ([IO.Path]::GetFileName($bakParent) -ne '__BACKUP_DIR__') { Stop-Here '這個還原檔不在遊戲資料夾的 __BACKUP_DIR__ 裡，無法確認要還原到哪裡。請不要搬動備份資料夾。沒有改動任何檔案。' 2 }
  $mc = [IO.Path]::GetDirectoryName($bakParent)
  Say "遊戲資料夾：$mc"
  if (-not (Test-IsGameDir $mc)) { Stop-Here '備份資料夾不在遊戲資料夾的 __BACKUP_DIR__ 裡（或遊戲資料夾沒有 mods），無法確認要還原到哪裡。沒有改動任何檔案。' 2 }
  Stop-IfGameRunning $mc '還原'

  $restore = @(); $delete = @(); $dirs = @(); $changed = @(); $broken = @(); $already = 0
  foreach ($row in (Read-Rows $manifest)) {
    if ($row.Count -ne 4 -or -not (Test-SafeRel $row[3] -Dir:($row[0] -eq 'newdir'))) { Stop-Here '備份清單有不正確的內容，為了安全不還原。沒有改動任何檔案。' 2 }
    $kind = $row[0]; $old = $row[1]; $new = $row[2]; $rel = $row[3]
    $dst = Join-Path $mc $rel
    if ($kind -eq 'overwritten') {
      $saved = Join-Path (Join-Path $BackupDir 'files') $rel
      if (-not (Test-Path -LiteralPath $saved -PathType Leaf) -or (Hash-File $saved) -ne $old) { $broken += $rel; continue }
      if (-not (Test-Path -LiteralPath $dst -PathType Leaf)) { $changed += "$rel（套用後被刪除）"; continue }
      $cur = Hash-File $dst
      if ($cur -eq $new) { $restore += [pscustomobject]@{ Rel = $rel; Saved = $saved; Dst = $dst; Old = $old } }
      elseif ($cur -eq $old) { $already++ }
      else { $changed += $rel }
    } elseif ($kind -eq 'added') {
      if (-not (Test-Path -LiteralPath $dst -PathType Leaf)) { $already++; continue }
      if ((Hash-File $dst) -eq $new) { $delete += [pscustomobject]@{ Rel = $rel; Dst = $dst } }
      else { $changed += $rel }
    } elseif ($kind -eq 'newdir') {
      $dirs += $rel
    } else {
      Stop-Here '備份清單有不認得的項目，為了安全不還原。沒有改動任何檔案。' 2
    }
  }

  Say ''
  Say "會放回原檔：$($restore.Count) 個；會刪除這次新增的檔：$($delete.Count) 個。"
  if ($changed.Count -gt 0) { Warn '下面這些檔在套用後又被改過，不會動它們：'; Show-Names '不動的檔' $changed }
  if ($broken.Count -gt 0) { Warn '下面這些檔的備份不見或已損壞，不會動它們：'; Show-Names '備份有問題' $broken }
  if (($restore.Count + $delete.Count) -eq 0) {
    Say '沒有需要還原的檔（可能已經還原過）。'
    Wait-Close
    exit 0
  }
  if (-not (Ask-Yes '要開始還原嗎？')) { Stop-Here '已取消，沒有改動任何檔案。' 1 }

  foreach ($r in $restore) {
    try {
      [IO.File]::Copy($r.Saved, $r.Dst, $true)
      if ((Hash-File $r.Dst) -ne $r.Old) { throw '放回後內容不對。' }
    } catch {
      Stop-Here ("還原到一半失敗：$($r.Rel)`n原因：" + $_.Exception.Message + "`n這個檔可能正被其他程式使用。關閉後再執行一次還原即可（已還原的檔不會重複處理）。") 2
    }
  }
  foreach ($d in $delete) {
    try { [IO.File]::Delete($d.Dst) } catch {
      Stop-Here ("刪除這次新增的檔時失敗：$($d.Rel)`n原因：" + $_.Exception.Message + "`n關閉占用的程式後再執行一次還原即可。") 2
    }
  }
  foreach ($d in @($dirs | Sort-Object -Property Length -Descending)) {
    $abs = Join-Path $mc $d
    if ((Test-Path -LiteralPath $abs -PathType Container) -and @(Get-ChildItem -LiteralPath $abs -Force).Count -eq 0) {
      try { [IO.Directory]::Delete($abs) } catch { Warn "資料夾刪不掉（不影響遊戲）：$d" }
    }
  }

  Say ''
  Say "完成：已放回 $($restore.Count) 個原檔，刪除 $($delete.Count) 個這次新增的檔。"
  if ($changed.Count -gt 0) { Warn "有 $($changed.Count) 個檔在套用後被改過，沒有動（清單在上面）。" }
  Say "備份資料夾保留在：`n  $BackupDir`n確認遊戲沒問題後，可以自行刪除。"
  Wait-Close
  exit 0
} catch {
  Stop-Here ("發生未預期的錯誤，已停止。`n原因：" + $_.Exception.Message) 2
}
"##;

pub(super) const README: &str = r##"模組包繁中翻譯 — 怎麼套用、怎麼還原

【套用】
1. 先關閉 Minecraft 和啟動器。
2. 執行自解檔，輸入下載頁上的解壓密碼。解壓後會自動開啟套用視窗。
3. 選你的 Minecraft 遊戲資料夾：裡面有 mods 資料夾的那一層
   （Prism／MultiMC 是實例裡的 .minecraft 或 minecraft）。
4. 套用視窗會先檢查，任何一項不通過就停下，不改任何檔：
   - 選的是不是遊戲資料夾（要有 mods，以及 options.txt 或 config）。
   - 遊戲有沒有開著（開著、或無法確認，都會停下）。
   - 分享檔是否完整（不完整請重新下載）。
5. 模組版本不同時：
   會列出「分享者有、你沒有」「你有、分享者沒有」「同名但大小不同」的模組，
   要你輸入 y 再按 Enter 才繼續；直接按 Enter 就取消。
   模組不同時翻譯可能對不上，任務與設定檔也可能換成分享者的版本（會先備份，可以還原）。
6. 覆蓋任何檔之前，會先把會被覆蓋的檔複製到：
     <遊戲資料夾>\__BACKUP_DIR__\<日期-時間>\
   並寫一份清單（清單.txt）。備份失敗就不會覆蓋任何檔。
7. 完成後開遊戲：選項 → 語言，選「繁體中文（台灣）」；資源包列表啟用「__PACK_ZIP__」。

【還原】
- 到 <遊戲資料夾>\__BACKUP_DIR__\<日期-時間>\，按兩下「__RESTORE_CMD__」。
- 還原會把備份的原檔放回、刪除這次新增的檔。
- 套用後又被改過的檔不會動，會列出來讓你自己決定。
- 有多次套用時，請從最新的一次開始還原（資料夾名稱的日期時間最新的那個），一次一個往前。
- 確認遊戲沒問題後，備份資料夾可以自行刪除。

【會放進遊戲資料夾的內容】
翻譯資源包，以及翻譯過的 config、kubejs、任務等檔案。完整清單見 __FILES_LIST__。
"##;
