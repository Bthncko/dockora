<#
.SYNOPSIS
  Dockora çökerse ya da öldürülürse Windows görev çubuğunu geri getirir.

.DESCRIPTION
  1. Çalışan Dockora'u kapatır (yoksa çubuğu yeniden gizler).
  2. Windows görev çubuklarını (ana + ikinci monitörler) yeniden gösterir ve
     dock'un verdiği boş pencere bölgesini kaldırır.
  3. Dock'un kaydettiği eski otomatik gizleme durumunu geri yükler
     (%APPDATA%\com.proxd.akillidock\taskbar-state.json). Kayıt yoksa
     otomatik gizlemeyi kapatır.
  4. -RestartExplorer verilirse Explorer'ı yeniden başlatır (ekranda ayrılmış
     alan takılı kaldıysa son çare).

  Çalıştırmak için:  powershell -ExecutionPolicy Bypass -File scripts\restore-taskbar.ps1

.PARAMETER RestartExplorer
  Explorer'ı yeniden başlatır.
#>
param([switch]$RestartExplorer)

$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class TaskbarRestore {
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT { public int Left, Top, Right, Bottom; }

  [StructLayout(LayoutKind.Sequential)]
  public struct APPBARDATA {
    public uint cbSize; public IntPtr hWnd; public uint uCallbackMessage;
    public uint uEdge; public RECT rc; public IntPtr lParam;
  }

  public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

  [DllImport("shell32.dll")] public static extern UIntPtr SHAppBarMessage(uint msg, ref APPBARDATA data);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int cmd);
  [DllImport("user32.dll")] public static extern int SetWindowRgn(IntPtr hWnd, IntPtr hRgn, bool redraw);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc fn, IntPtr lParam);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hWnd, System.Text.StringBuilder s, int n);

  public const uint ABM_GETSTATE = 4, ABM_SETSTATE = 10;
  public const int SW_SHOW = 5;

  public static List<IntPtr> TaskbarWindows() {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => {
      var sb = new System.Text.StringBuilder(64);
      GetClassNameW(h, sb, 64);
      var c = sb.ToString();
      if (c == "Shell_TrayWnd" || c == "Shell_SecondaryTrayWnd") list.Add(h);
      return true;
    }, IntPtr.Zero);
    return list;
  }

  public static void SetState(IntPtr tray, uint state) {
    var d = new APPBARDATA();
    d.cbSize = (uint)Marshal.SizeOf(typeof(APPBARDATA));
    d.hWnd = tray;
    d.lParam = (IntPtr)state;
    SHAppBarMessage(ABM_SETSTATE, ref d);
  }

  public static uint GetState() {
    var d = new APPBARDATA();
    d.cbSize = (uint)Marshal.SizeOf(typeof(APPBARDATA));
    return (uint)SHAppBarMessage(ABM_GETSTATE, ref d);
  }
}
'@

$stateFile = Join-Path $env:APPDATA 'com.proxd.akillidock\taskbar-state.json'

# 1) Dock açıksa kapat; yoksa çubuğu hemen yeniden gizler.
$dock = Get-Process -Name 'akilli-dock' -ErrorAction SilentlyContinue
if ($dock) {
  Write-Host "Dockora kapatılıyor (PID $($dock.Id -join ', '))..."
  $dock | Stop-Process -Force
  Start-Sleep -Milliseconds 500
}

# 2) Görev çubuklarını göster (dock'un verdiği boş pencere bölgesini de kaldır).
$bars = [TaskbarRestore]::TaskbarWindows()
if ($bars.Count -eq 0) {
  Write-Warning 'Görev çubuğu penceresi bulunamadı (Explorer çalışmıyor olabilir). -RestartExplorer deneyin.'
}
foreach ($h in $bars) {
  [void][TaskbarRestore]::SetWindowRgn($h, [IntPtr]::Zero, $true)
  [void][TaskbarRestore]::ShowWindow($h, [TaskbarRestore]::SW_SHOW)
}

# 3) Otomatik gizleme durumunu geri yükle.
$autohide = $false
if (Test-Path $stateFile) {
  try {
    $saved = Get-Content $stateFile -Raw | ConvertFrom-Json
    $autohide = [bool]$saved.autohide
    Write-Host "Kayıtlı durum bulundu: otomatik gizleme = $autohide"
  } catch {
    Write-Warning "Durum dosyası okunamadı: $_  — otomatik gizleme kapatılıyor."
  }
}
$tray = [TaskbarRestore]::FindWindowW('Shell_TrayWnd', $null)
if ($tray -ne [IntPtr]::Zero) {
  $state = if ($autohide) { 1 } else { 2 }   # ABS_AUTOHIDE = 1, ABS_ALWAYSONTOP = 2
  [TaskbarRestore]::SetState($tray, $state)
}
if (Test-Path $stateFile) { Remove-Item $stateFile -Force }

# 4) Son çare: Explorer'ı yeniden başlat (takılı kalan ayrılmış alanı temizler).
if ($RestartExplorer) {
  Write-Host 'Explorer yeniden başlatılıyor...'
  Get-Process -Name explorer -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 1
  if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
}

$visible = ($bars | Where-Object { [TaskbarRestore]::IsWindowVisible($_) }).Count
$nowAutohide = ([TaskbarRestore]::GetState() -band 1) -ne 0
Write-Host "Bitti. Görünen görev çubuğu: $visible/$($bars.Count), otomatik gizleme: $nowAutohide"
