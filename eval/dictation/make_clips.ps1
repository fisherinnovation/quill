# Windows counterpart of make_clips.sh: builds clips/<id>-<voice>.wav from
# utterances.txt with Windows text-to-speech (SAPI), once in each installed
# English voice (David and Zira on a stock Windows 11), with 0.4 s of
# silence before and 0.8 s after, or the end cut off when `cut` > 0. 16 kHz
# mono 16-bit. Also writes clips/<id>-<voice>.txt, the reference
# `quill-cli dictate` scores against.
#
#   powershell -ExecutionPolicy Bypass -File eval/dictation/make_clips.ps1
#
# -Utterances and -Out read another list in the same format and write
# elsewhere.
#
# Synthetic speech: good for latency and for catching hallucinations, not
# for judging accuracy on real voices.
param(
    [string]$Utterances = (Join-Path $PSScriptRoot 'utterances.txt'),
    [string]$Out = (Join-Path $PSScriptRoot 'clips')
)
Add-Type -AssemblyName System.Speech
$clips = $Out
New-Item -ItemType Directory -Force $clips | Out-Null

$rate = 16000
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo($rate, 'Sixteen', 'Mono')
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$voices = $synth.GetInstalledVoices() |
    Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -like 'en-*' } |
    ForEach-Object { $_.VoiceInfo.Name }

# A WAV file of 16-bit PCM: `lead` seconds of silence, then `speech`
# (bytes) up to `keep` bytes, then `tail` seconds of silence.
function Write-Wav([string]$path, [byte[]]$speech, [int]$keep, [double]$lead, [double]$tail) {
    $leadBytes = [int]($rate * $lead) * 2
    $tailBytes = [int]($rate * $tail) * 2
    $length = $leadBytes + $keep + $tailBytes
    $out = [System.IO.File]::Create($path)
    $w = New-Object System.IO.BinaryWriter($out)
    $w.Write([Text.Encoding]::ASCII.GetBytes('RIFF')); $w.Write([int](36 + $length))
    $w.Write([Text.Encoding]::ASCII.GetBytes('WAVEfmt ')); $w.Write([int]16)
    $w.Write([int16]1); $w.Write([int16]1); $w.Write([int]$rate); $w.Write([int]($rate * 2))
    $w.Write([int16]2); $w.Write([int16]16)
    $w.Write([Text.Encoding]::ASCII.GetBytes('data')); $w.Write([int]$length)
    $w.Write((New-Object byte[] $leadBytes))
    $w.Write($speech, 0, $keep)
    $w.Write((New-Object byte[] $tailBytes))
    $w.Close()
}

$utf8 = New-Object System.Text.UTF8Encoding($false)
$count = 0
foreach ($line in Get-Content $Utterances) {
    if ($line -match '^#' -or -not $line.Trim()) { continue }
    $id, $null, $cut, $text, $expected = $line -split '\|', 5
    foreach ($voice in $voices) {
        $synth.SelectVoice($voice)
        $stream = New-Object System.IO.MemoryStream
        $synth.SetOutputToAudioStream($stream, $format)
        $synth.Speak($text)
        $synth.SetOutputToNull()
        $speech = $stream.ToArray()
        $name = '{0}-{1}' -f $id, ($voice -split ' ')[1].ToLower()
        $wav = Join-Path $clips "$name.wav"
        if ([double]$cut -gt 0) {
            $keep = [Math]::Max(0, $speech.Length - [int]($rate * [double]$cut) * 2)
            Write-Wav $wav $speech $keep 0.4 0.05
        } else {
            Write-Wav $wav $speech $speech.Length 0.4 0.8
        }
        [System.IO.File]::WriteAllText((Join-Path $clips "$name.txt"), "$expected`n", $utf8)
        $count++
    }
}
"wrote $count clips to $clips"
