param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $MsiPath -PathType Leaf)) {
    throw 'MSI package does not exist.'
}

# Open the built package read-only. This checks what WiX actually emitted,
# including Handlebars rendering, without installing or modifying the MSI.
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @($MsiPath, 0))

function Read-MsiTable([string]$Table, [string[]]$Columns) {
    $tick = [char]96
    $quoted = ($Columns | ForEach-Object { '{0}{1}{0}' -f $tick, $_ }) -join ', '
    $query = 'SELECT ' + $quoted + ' FROM ' + $tick + $Table + $tick
    $view = $database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $database, @($query))
    try {
        [void]$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null)
        while ($record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)) {
            $row = [ordered]@{}
            for ($i = 0; $i -lt $Columns.Count; $i++) {
                $row[$Columns[$i]] = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, @($i + 1))
            }
            [pscustomobject]$row
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($record)
        }
    }
    finally {
        [void]$view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null)
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($view)
    }
}

try {
    $properties = @(Read-MsiTable 'Property' @('Property', 'Value'))
    $product = @($properties | Where-Object { $_.Property -ceq 'ProductName' })
    $manufacturer = @($properties | Where-Object { $_.Property -ceq 'Manufacturer' })
    if ($product.Count -ne 1 -or -not $product[0].Value) { throw 'MSI product name is missing.' }
    if ($manufacturer.Count -ne 1 -or -not $manufacturer[0].Value) { throw 'MSI manufacturer is missing.' }

    $legacyKey = 'Software\' + $manufacturer[0].Value + '\' + $product[0].Value
    $searches = @(
        @{ Property = 'CCOWNMSIINSTALLDIR'; Signature = 'OwnMsiInstallDir'; Key = 'Software\WQMYH\MultiCompatCC\Installer'; Name = 'InstallDir' }
        @{ Property = 'CCPREVMSIINSTALLDIR'; Signature = 'PrevInstallDirWithName'; Key = $legacyKey; Name = 'InstallDir' }
        @{ Property = 'CCPREVNSISINSTALLDIR'; Signature = 'PrevInstallDirNoName'; Key = $legacyKey; Name = '' }
    )
    $appSearch = @(Read-MsiTable 'AppSearch' @('Property', 'Signature_'))
    $regLocator = @(Read-MsiTable 'RegLocator' @('Signature_', 'Root', 'Key', 'Name', 'Type'))
    foreach ($search in $searches) {
        $app = @($appSearch | Where-Object { $_.Property -ceq $search.Property -and $_.Signature_ -ceq $search.Signature })
        $registry = @($regLocator | Where-Object { $_.Signature_ -ceq $search.Signature })
        if ($app.Count -ne 1 -or $registry.Count -ne 1 -or $registry[0].Root -ne '1' -or $registry[0].Key -cne $search.Key -or $registry[0].Name -cne $search.Name -or $registry[0].Type -ne '18') {
            throw "Unexpected MSI registry search for $($search.Property)."
        }
    }

    $conditions = [ordered]@{
        SetOwnMsiInstallDir = 'NOT INSTALLDIR AND CCOWNMSIINSTALLDIR'
        SetPreviousMsiInstallDir = 'NOT INSTALLDIR AND CCPREVMSIINSTALLDIR'
        SetPreviousNsisInstallDir = 'NOT INSTALLDIR AND CCPREVNSISINSTALLDIR'
        SetDefaultInstallDir = 'NOT INSTALLDIR'
    }
    $expectedOrder = @('AppSearch') + @($conditions.Keys) + @('CostInitialize', 'CostFinalize')
    foreach ($table in @('InstallUISequence', 'InstallExecuteSequence')) {
        $rows = @(Read-MsiTable $table @('Action', 'Condition', 'Sequence'))
        $previous = 0
        foreach ($action in $expectedOrder) {
            $found = @($rows | Where-Object { $_.Action -ceq $action })
            if ($found.Count -ne 1 -or [int]$found[0].Sequence -le $previous) {
                throw "Unexpected $table sequence for $action."
            }
            if ($conditions.Contains($action) -and $found[0].Condition -cne $conditions[$action]) {
                throw "Unexpected $table condition for $action."
            }
            if (-not $conditions.Contains($action) -and $found[0].Condition) {
                throw "Standard MSI action $action must not be conditional."
            }
            $previous = [int]$found[0].Sequence
        }
    }

    $targets = [ordered]@{
        SetOwnMsiInstallDir = '[CCOWNMSIINSTALLDIR]'
        SetPreviousMsiInstallDir = '[CCPREVMSIINSTALLDIR]'
        SetPreviousNsisInstallDir = '[CCPREVNSISINSTALLDIR]'
        SetDefaultInstallDir = ('[LocalAppDataFolder]Programs\' + $product[0].Value)
    }
    $actions = @(Read-MsiTable 'CustomAction' @('Action', 'Type', 'Source', 'Target'))
    foreach ($action in $targets.Keys) {
        $found = @($actions | Where-Object { $_.Action -ceq $action })
        if ($found.Count -ne 1 -or $found[0].Type -ne '307' -or $found[0].Source -cne 'INSTALLDIR' -or $found[0].Target -cne $targets[$action]) {
            throw "Unexpected MSI directory assignment for $action."
        }
    }

    $directories = @(Read-MsiTable 'Directory' @('Directory', 'Directory_Parent', 'DefaultDir'))
    foreach ($item in @(@('INSTALLDIR', 'LocalAppDataFolder'), @('ApplicationProgramsFolder', 'ProgramMenuFolder'))) {
        $found = @($directories | Where-Object { $_.Directory -ceq $item[0] })
        if ($found.Count -ne 1 -or $found[0].Directory_Parent -cne $item[1]) {
            throw "Unexpected MSI directory parent for $($item[0])."
        }
        # MSI stores a possible short|long name and an optional :source name.
        $targetName = (($found[0].DefaultDir -split ':', 2)[0] -split '\|', 2)[-1]
        if ($targetName -cne $product[0].Value) { throw "Unexpected MSI directory name for $($item[0])." }
    }
    if (@($directories | Where-Object { $_.Directory -ceq 'TauriLocalAppDataPrograms' }).Count -ne 0) { throw 'MSI must not own the shared Programs directory.' }
    $removals = @(Read-MsiTable 'RemoveFile' @('FileName', 'DirProperty', 'InstallMode'))
    if ($removals.Count -ne 2) { throw 'Unexpected MSI removal entries.' }
    foreach ($directory in @('INSTALLDIR', 'ApplicationProgramsFolder')) {
        $found = @($removals | Where-Object { $_.DirProperty -ceq $directory -and -not $_.FileName -and $_.InstallMode -eq '2' })
        if ($found.Count -ne 1) { throw "Unexpected MSI removal policy for $directory." }
    }
    Write-Host 'Verified MSI directory assignments, sequence and empty-folder removal policy.'
}
finally {
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($database)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
}
