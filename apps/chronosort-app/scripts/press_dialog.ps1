param([string]$Title, [string]$Button, [int]$TimeoutSec = 10)
# Finds a top-level window with this title and presses the named button (UI Automation).
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::RootElement
$deadline = (Get-Date).AddSeconds($TimeoutSec)
while ((Get-Date) -lt $deadline) {
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Title)
    $win = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if ($win) {
        $text = ($win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
            Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Text } | ForEach-Object { $_.Current.Name }) -join ' / '
        "DIALOG TEXT: $text"
        $btn = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
            Where-Object { ($_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -or $_.Current.ClassName -eq 'CCPushButton') -and $_.Current.Name -eq $Button } |
            Select-Object -First 1
        if (-not $btn) {
            "button '$Button' not found; buttons: " + (($win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
                Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button } | ForEach-Object { $_.Current.Name }) -join ', ')
            exit 1
        }
        $h = $btn.Current.NativeWindowHandle
        if ($h) {
            # Task-dialog buttons (CCPushButton) have no Invoke pattern: send BM_CLICK.
            Add-Type -Namespace W -Name U -MemberDefinition '[DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);'
            [void][W.U]::SendMessage([IntPtr]$h, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        } else {
            $btn.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        }
        "pressed '$Button'"
        exit 0
    }
    Start-Sleep -Milliseconds 300
}
"no dialog '$Title' within $TimeoutSec s"; exit 1
