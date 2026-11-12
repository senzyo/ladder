# This script runs in Windows PowerShell, before the desktop app opens the machine journal.
$ErrorActionPreference = 'Stop'
# Return the underlying Windows error without including external text or script source.
trap { exit $_.Exception.GetBaseException().HResult }
$path = $env:LADDER_DNS_RECOVERY_DIRECTORY
$aces = '(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;OW)'
function Sorted-Aces([string]$sddl) {
    # NTFS can reorder our three allow ACEs. Keep trustee, permissions and flags exact.
    $entries = [regex]::Split(($sddl -replace '^D:[A-Z]*', ''), '(?<=\))(?=\()')
    [Array]::Sort($entries, [StringComparer]::Ordinal)
    [string]::Concat($entries)
}
if (![IO.Directory]::Exists($path)) {
    $security = [Security.AccessControl.DirectorySecurity]::new()
    $security.SetSecurityDescriptorSddlForm('O:BAD:P' + $aces)
    [void][IO.Directory]::CreateDirectory($path, $security)
}

# OWNER RIGHTS is read-only, including for children owned by the individual admin account.
$reparse = [IO.FileAttributes]::ReparsePoint
if (([IO.File]::GetAttributes($path) -band $reparse) -ne 0) { exit 2 }
$access = [Security.AccessControl.AccessControlSections]::Access
$acl = [IO.Directory]::GetAccessControl($path)
if (!$acl.AreAccessRulesProtected -or
    (Sorted-Aces $acl.GetSecurityDescriptorSddlForm($access)) -cne (Sorted-Aces $aces)) { exit 3 }

$allowed = @('ladder-dns-recovery.exe', 'ladder-dns-recovery.tmp', 'dns-task.xml',
             'dns-session.lock', 'dns-recovery.json', 'dns-recovery.tmp')
$fileAces = $aces.Replace(';OICI;', ';;')
foreach ($file in [IO.Directory]::EnumerateFileSystemEntries($path)) {
    # A boot recovery finishing concurrently can have just removed its journal.
    try {
        if ([IO.Path]::GetExtension($file) -eq '.bak') { continue }
        if ($allowed -notcontains [IO.Path]::GetFileName($file) -or
            ([IO.File]::GetAttributes($file) -band ($reparse -bor [IO.FileAttributes]::Directory)) -ne 0) { exit 4 }
        $sddl = [IO.File]::GetAccessControl($file).GetSecurityDescriptorSddlForm($access)
        if ((Sorted-Aces $sddl.Replace(';ID;', ';;')) -cne (Sorted-Aces $fileAces)) { exit 5 }
    } catch [IO.FileNotFoundException] {
        continue
    }
}
