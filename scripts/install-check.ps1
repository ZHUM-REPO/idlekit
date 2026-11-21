# Installs IdleKit from a setup file without asking anything, checks what
# landed on the machine, starts the installed app and looks for its window,
# then uninstalls it again.
#
# Meant for a throwaway Windows machine such as the build machine: it installs
# and removes IdleKit for the current user.
#
#   pwsh scripts/install-check.ps1 -Installer path\to\IdleKit_0.2.0_x64-setup.exe [-Version 0.2.0]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"
$product = "IdleKit"
$identifier = "dev.idlekit.app"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
$problems = [System.Collections.Generic.List[string]]::new()
$passed = [System.Collections.Generic.List[string]]::new()

function Problem([string]$text) {
    $problems.Add($text)
    Write-Output "::error::$text"
}

function Passed([string]$text) {
    $passed.Add($text)
    Write-Output "ok: $text"
}

# Starts a program and waits for it to end. Gives its exit code, or nothing
# when it had to be stopped for taking too long.
function Invoke-Timed([string]$file, [string[]]$arguments, [int]$seconds) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru
    $null = $process.Handle
    if (-not $process.WaitForExit($seconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        return $null
    }
    return $process.ExitCode
}

function Finish {
    Write-Output ""
    if ($env:GITHUB_STEP_SUMMARY) {
        $lines = @("### Install check: $(Split-Path -Leaf $Installer)", "")
        $lines += @($passed | ForEach-Object { "- ok: $_" })
        $lines += @($problems | ForEach-Object { "- **problem:** $_" })
        Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value $lines
    }
    if ($problems.Count -gt 0) {
        Write-Output "Install check failed: $($problems.Count) problem(s)."
        exit 1
    }
    Write-Output "Install check passed."
    exit 0
}

# Every top-level window of one process: Process.MainWindowTitle is no use
# here, because it can pick an invisible helper window instead of the app's.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class IdleKitWindows
{
    private delegate bool EnumProc(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumProc callback, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public sealed class Info
    {
        public string Title;
        public bool Visible;
        public int Width;
        public int Height;
    }

    public static List<Info> Of(int processId)
    {
        var found = new List<Info>();
        EnumWindows((window, parameter) =>
        {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == (uint)processId)
            {
                var text = new StringBuilder(512);
                GetWindowText(window, text, text.Capacity);
                Rect rect;
                GetWindowRect(window, out rect);
                found.Add(new Info
                {
                    Title = text.ToString(),
                    Visible = IsWindowVisible(window),
                    Width = rect.Right - rect.Left,
                    Height = rect.Bottom - rect.Top
                });
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

# --- the setup file ---------------------------------------------------------

$Installer = (Resolve-Path -LiteralPath $Installer).Path
$setup = Get-Item -LiteralPath $Installer
Write-Output "Setup file: $($setup.Name), $($setup.Length) bytes"
Write-Output "SHA-256:    $((Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash)"
Write-Output ""

if (Test-Path -LiteralPath $uninstallKey) {
    Write-Output "IdleKit is already installed on this machine; the check installs over it."
}

# --- install ----------------------------------------------------------------

Write-Output "Installing (silent)..."
$code = Invoke-Timed $Installer @("/S") 300
if ($null -eq $code) {
    Problem "The installer was still running after 5 minutes and was stopped"
    Finish
}
if ($code -ne 0) {
    Problem "The installer ended with exit code $code"
    Finish
}
Passed "the installer ran to the end without asking anything"

$entry = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
if (-not $entry) {
    Problem "Windows has no 'installed apps' entry for $product after installing"
    Finish
}
$dir = ([string]$entry.InstallLocation).Trim('"')
Write-Output "Installed to: $dir (version $($entry.DisplayVersion))"
if ($Version -and $entry.DisplayVersion -ne $Version) {
    Problem "Installed version is $($entry.DisplayVersion), expected $Version"
} else {
    Passed "listed under installed apps as $product $($entry.DisplayVersion)"
}

$missing = @()
foreach ($name in "idlekit.exe", "steam-helper.exe", "steam_api64.dll", "uninstall.exe") {
    if (-not (Test-Path -LiteralPath (Join-Path $dir $name))) { $missing += $name }
}
Get-ChildItem -LiteralPath $dir | ForEach-Object { "  {0,12:N0}  {1}" -f $_.Length, $_.Name }
if ($missing.Count -gt 0) {
    Problem "Missing from the install folder: $($missing -join ', ')"
    Finish
}
Passed "the app, the Steam helper and Steam's library are in the install folder"

$exe = Join-Path $dir "idlekit.exe"
$shell = New-Object -ComObject WScript.Shell
$programs = [Environment]::GetFolderPath("Programs")
$startMenu = @((Join-Path $programs "$product.lnk"), (Join-Path $programs "$product\$product.lnk")) |
    Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $startMenu) {
    Problem "No Start menu shortcut was created"
} elseif ($shell.CreateShortcut($startMenu).TargetPath -ne $exe) {
    Problem "The Start menu shortcut points at $($shell.CreateShortcut($startMenu).TargetPath), not at $exe"
} else {
    Passed "the Start menu shortcut opens the installed app"
}
$desktopShortcut = Join-Path ([Environment]::GetFolderPath("Desktop")) "$product.lnk"
Write-Output "Desktop shortcut: $(if (Test-Path -LiteralPath $desktopShortcut) { 'created' } else { 'none' })"

# --- start the installed app ------------------------------------------------

# IdleKit keeps a log file from 0.2.2 on.
$logFile = Join-Path $env:APPDATA "$identifier\log.txt"
$loadedLine = "The window loaded"
$logged = $false
try { $logged = [version]$entry.DisplayVersion -ge [version]"0.2.2" } catch { $logged = $false }
function Read-Log {
    # The app has the file open for writing, so read it without locking it.
    try {
        $stream = [IO.File]::Open($logFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { return ([IO.StreamReader]::new($stream, [Text.Encoding]::UTF8)).ReadToEnd() } finally { $stream.Dispose() }
    } catch {
        return ""
    }
}
if ($logged -and (Test-Path -LiteralPath $logFile)) { Remove-Item -LiteralPath $logFile -Force }

Write-Output ""
Write-Output "Starting the installed app..."
$app = Start-Process -FilePath $exe -PassThru
$null = $app.Handle
$window = $null
$deadline = (Get-Date).AddSeconds(45)
while ((Get-Date) -lt $deadline -and -not $app.HasExited) {
    Start-Sleep -Seconds 2
    $window = [IdleKitWindows]::Of($app.Id) |
        Where-Object { $_.Visible -and $_.Title -eq $product -and $_.Width -ge 300 -and $_.Height -ge 200 } |
        Select-Object -First 1
    if ($window) { break }
}

if ($app.HasExited) {
    Problem "IdleKit stopped by itself shortly after starting (exit code $($app.ExitCode))"
} else {
    Write-Output "Windows of the app:"
    [IdleKitWindows]::Of($app.Id) | ForEach-Object {
        "  '{0}'  {1}  {2}x{3}" -f $_.Title, $(if ($_.Visible) { "visible" } else { "hidden" }), $_.Width, $_.Height
    }
    if ($window) {
        Passed "the installed app opened its window ($($window.Width)x$($window.Height))"
    } else {
        Problem "IdleKit is running but showed no window titled '$product' within 45 seconds"
    }

    $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($app.Id)" | Select-Object -ExpandProperty Name)
    Write-Output "Started by the app: $(if ($children.Count) { ($children | Sort-Object -Unique) -join ', ' } else { 'nothing' })"
    if ($children -contains "msedgewebview2.exe") {
        Passed "the window's web engine (WebView2) started"
    } else {
        Write-Output "::warning::No WebView2 process was found under the app"
    }

    # Opening it a second time should hand over to the copy already running.
    $second = Start-Process -FilePath $exe -PassThru
    $null = $second.Handle
    if ($second.WaitForExit(20000)) {
        if ($app.HasExited) {
            Problem "Opening IdleKit a second time closed the first copy"
        } else {
            Passed "opening it a second time hands over to the copy already running"
        }
    } else {
        Stop-Process -Id $second.Id -Force -ErrorAction SilentlyContinue
        Problem "A second copy of IdleKit stayed open next to the first"
    }

    # The window writes a line to the log once its contents are up.
    if ($logged) {
        $deadline = (Get-Date).AddSeconds(60)
        while ((Get-Date) -lt $deadline -and -not (Read-Log).Contains($loadedLine)) { Start-Sleep -Seconds 2 }
    }

    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
    $null = $app.WaitForExit(15000)
}
Get-Process -Name "idlekit", "steam-helper" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

foreach ($folder in "$env:APPDATA\$identifier", "$env:LOCALAPPDATA\$identifier") {
    $names = @(Get-ChildItem -LiteralPath $folder -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name)
    Write-Output "$folder holds: $(if ($names.Count) { $names -join ', ' } else { 'nothing' })"
}

# --- the log file -----------------------------------------------------------

if ($logged) {
    Write-Output ""
    $log = Read-Log
    if (-not $log) {
        Problem "The app wrote no log file at $logFile"
    } else {
        Write-Output "log.txt ($((Get-Item -LiteralPath $logFile).Length) bytes):"
        $log -split "`n" | ForEach-Object { "  $($_.TrimEnd())" }
        if ($log -match "======== IdleKit \S+ started on windows") {
            Passed "the app keeps a log file (log.txt) and each run starts with its version and system"
        } else {
            Problem "log.txt has no line saying which version started on which system"
        }
        if ($log.Contains($loadedLine)) {
            Passed "the window's contents loaded and what happens in the window reaches log.txt"
        } else {
            Problem "The window opened but its contents had not loaded after a minute (nothing from the window in log.txt)"
        }
        $private = @($env:USERNAME, $env:COMPUTERNAME) | Where-Object { $_ -and $_.Length -ge 3 }
        $found = @($private | Where-Object { $log.IndexOf($_, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
        if ($found.Count -gt 0) {
            Problem "log.txt contains this machine's user or computer name"
        } else {
            Passed "log.txt holds neither the Windows user name nor the computer name"
        }
    }
} else {
    Write-Output "This version is from before the log file (0.2.2), so there is no log.txt to check."
}

# --- uninstall --------------------------------------------------------------

Write-Output ""
Write-Output "Uninstalling (silent)..."
# "_?=" keeps the uninstaller in the foreground so it can be waited for; the
# uninstaller itself then stays behind in the otherwise emptied folder.
$before = $problems.Count
$code = Invoke-Timed (Join-Path $dir "uninstall.exe") @("/S", "_?=$dir") 300
if ($null -eq $code) {
    Problem "The uninstaller was still running after 5 minutes and was stopped"
} elseif ($code -ne 0) {
    Problem "The uninstaller ended with exit code $code"
} else {
    $left = @()
    foreach ($name in "idlekit.exe", "steam-helper.exe", "steam_api64.dll") {
        if (Test-Path -LiteralPath (Join-Path $dir $name)) { $left += $name }
    }
    if ($left.Count -gt 0) { Problem "Still there after uninstalling: $($left -join ', ')" }
    if (Test-Path -LiteralPath $uninstallKey) { Problem "Still listed under installed apps after uninstalling" }
    if ($startMenu -and (Test-Path -LiteralPath $startMenu)) { Problem "The Start menu shortcut is still there after uninstalling" }
    if (Test-Path -LiteralPath $desktopShortcut) { Problem "The desktop shortcut is still there after uninstalling" }
    if ($problems.Count -eq $before) { Passed "uninstalling removed the app, its shortcuts and its installed-apps entry" }
}

Finish
