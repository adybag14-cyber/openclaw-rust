param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$defaultTests = @(
  "bridge::tests::steer_mode_keeps_latest_pending_at_bridge_level",
  "bridge::tests::followup_queue_pressure_preserves_order_without_duplicates",
  "bridge::tests::session_routing_corpus_matches_expected_delivery_order",
  "bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates",
  "bridge::tests::reply_back_payload_preserves_group_and_direct_delivery_context",
  "gateway::tests::dispatcher_list_supports_label_spawn_filters_and_message_hints",
  "gateway::tests::dispatcher_resolve_supports_label_agent_and_spawn_filters"
)

$sqliteTests = @(
  "state::tests::sqlite_state_survives_restart_and_continues_counters",
  "state::tests::sqlite_state_recovers_multiple_sessions_after_restart"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

foreach ($testName in $defaultTests) {
  Write-Output "[parity] running CP2 fixture: $testName"
  if ($toolchainArg) {
    & $CargoCommand $toolchainArg test $testName -- --nocapture
  } else {
    & $CargoCommand test $testName -- --nocapture
  }
  if ($LASTEXITCODE -ne 0) {
    throw "CP2 fixture failed: $testName (exit $LASTEXITCODE)"
  }
}

foreach ($testName in $sqliteTests) {
  Write-Output "[parity] running CP2 sqlite fixture: $testName"
  if ($toolchainArg) {
    & $CargoCommand $toolchainArg test --features sqlite-state $testName -- --nocapture
  } else {
    & $CargoCommand test --features sqlite-state $testName -- --nocapture
  }
  if ($LASTEXITCODE -ne 0) {
    throw "CP2 sqlite fixture failed: $testName (exit $LASTEXITCODE)"
  }
}

Write-Output "[parity] CP2 gate passed"
