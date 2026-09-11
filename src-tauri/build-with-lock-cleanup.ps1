# Tauri 編譯腳本 - 自動清理檔案鎖定
param(
    [switch]$Release
)

$buildMode = if ($Release) { "--release" } else { "" }
$targetDir = if ($Release) { "release" } else { "debug" }

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Tauri 編譯腳本 (自動清理鎖檔)" -ForegroundColor Cyan
Write-Host "模式: $(if ($Release) { 'Release' } else { 'Debug' })" -ForegroundColor Yellow
Write-Host "========================================`n" -ForegroundColor Cyan

# 啟動編譯
$buildJob = Start-Job -ScriptBlock {
    param($mode)
    Set-Location "E:\moyul\Desktop\ReversedFront\src-tauri"
    cargo build $mode 2>&1
} -ArgumentList $buildMode

Write-Host "編譯已開始，背景監控鎖檔清理中...`n" -ForegroundColor Green

# 監控並清理鎖檔
$iteration = 0
while ($buildJob.State -eq 'Running') {
    Start-Sleep -Seconds 5
    $iteration++
    
    # 清理 .cargo-lock
    $lockPath = "E:\moyul\Desktop\ReversedFront\src-tauri\target\$targetDir\.cargo-lock"
    if (Test-Path $lockPath) {
        try {
            Remove-Item $lockPath -Force -ErrorAction Stop
            Write-Host "[$iteration] 已清理 .cargo-lock" -ForegroundColor Yellow
        } catch {
            Write-Host "[$iteration] .cargo-lock 被佔用，跳過" -ForegroundColor Gray
        }
    }
    
    # 每 30 秒顯示狀態
    if ($iteration % 6 -eq 0) {
        $elapsed = $iteration * 5
        Write-Host "[$iteration] 編譯進行中... (已運行 ${elapsed}秒)" -ForegroundColor Cyan
    }
}

# 獲取編譯結果
$result = Receive-Job $buildJob
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "編譯完成!" -ForegroundColor Green
Write-Host "========================================`n" -ForegroundColor Cyan

# 顯示最後 50 行輸出
$result | Select-Object -Last 50

# 檢查執行檔
$exePath = "E:\moyul\Desktop\ReversedFront\src-tauri\target\$targetDir\reversed-front.exe"
if (Test-Path $exePath) {
    $exeInfo = Get-Item $exePath
    Write-Host "`n✅ 編譯成功!" -ForegroundColor Green
    Write-Host "執行檔: $exePath" -ForegroundColor Cyan
    Write-Host "大小: $([math]::Round($exeInfo.Length / 1MB, 2)) MB" -ForegroundColor Cyan
    Write-Host "修改時間: $($exeInfo.LastWriteTime)" -ForegroundColor Cyan
} else {
    Write-Host "`n❌ 編譯失敗 - 未找到執行檔" -ForegroundColor Red
    exit 1
}
