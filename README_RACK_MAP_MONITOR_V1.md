# BlockOps Static IP Tool - Rack Map Monitor v1

This build adds the first version of the Hashcore-style rack map workflow.

## New workflow

1. Use dashboard mode to click a slot and view its details.
2. Turn on `Edit mode` only when you want clicks to arm a target IP.
3. Start the IP Report listener.
4. Set the rack map rule:
   - `Rack 1 Slot 1 IP`
   - number of racks
   - rack size, usually `156` or `168`
5. In edit mode, click a square on the Rack Map.
6. Press IP Report on the physical miner in that slot.
7. The app captures the reported current IP/MAC and maps it to the clicked target IP.
8. If `Auto apply armed reports` is enabled, the app immediately uses the existing safe apply flow.

## Rack IP rule

The first version uses a simple rule:

- Rack 1 Slot 1 is the IP typed in the map header.
- Each rack increments the third octet.
- Each slot becomes the fourth octet.

Example:

- Rack 1 Slot 1 IP: `10.4.1.1`
- Rack count: `19`
- Rack size: `168`

This maps Rack 1 to `10.4.1.1-168`, Rack 2 to `10.4.2.1-168`, and continues through Rack 19 at `10.4.19.1-168`.

## Resilient monitoring

- Type a rack number and click `Scan Rack` or press Enter to scan only that rack.
- A previously online miner must fail two checks in a row before the dashboard marks it offline.
- Individual connection failures do not stop the rest of the scan or the live monitor.
- `Stop Live` cancels between network probes and ignores results that arrive after cancellation.
- `Rescan All` starts a fresh scan across all 19 racks.
- Apply batches cannot overlap; IP Reports received during an apply are queued for the next batch.
- Successful apply steps immediately update the miner's current IP and final status.
- Known VNISH and Bitmain miners use their matching setter directly instead of waiting through the wrong firmware method.
- Rack 1 Slot 77: `10.5.9.77`
- Rack 2 Slot 77: `10.5.10.77`

This can be changed later if the real site map needs a different rule.

## Monitor

The monitor scan is a read-only layered discovery pass:

- First it checks TCP port `80` and `22` with short timeouts.
- If port `80` is open, it probes VNISH endpoints:
  - `/api/v1/status`
  - `/api/v1/info`
  - `/api/v1/model`
  - `/api/v1/summary`
- If VNISH does not fingerprint, it probes Bitmain/stock CGI endpoints:
  - `/cgi-bin/get_network_info.cgi`
  - `/cgi-bin/get_system_info.cgi`
  - `/cgi-bin/summary.cgi`
  - `/cgi-bin/minerStatus.cgi`
  - `/cgi-bin/set_network_conf.cgi`
- Scans run in parallel using the app's `Parallel jobs` setting, capped at 128 checks at once.
- `Scan Once` runs one pass.
- `Start Live` keeps scanning on the configured live interval until `Stop Live` is pressed.
- Bright blue means VNISH miner.
- Teal means Bitmain/stock miner.
- Purple means miner web/API exists but auth is required.
- Muted blue means web is online but not confirmed as a miner.
- Slate means SSH only.
- Red means offline.
- Gray means unknown/not scanned.
- Green means a captured row is already on its correct target IP.
- Yellow means a captured row still needs to be applied.

This discovery is intentionally read-only. It does not unlock, restart, configure, or change miners.

## Layout update

The rack map shows two racks per row instead of a single vertical stack or a long left-to-right strip. This cuts down scrolling while keeping nearby racks easy to compare.

## Dashboard vs edit mode

Normal dashboard mode is safe for viewing. Clicking a square only selects it and shows its target IP, monitor state, captured current IP, MAC, and apply status.

Dashboard clicks also open a miner detail popup. The popup tries to read VNISH `/api/v1/...` and Bitmain `/cgi-bin/...` status/detail endpoints and displays hashrate, power, efficiency, uptime, temperature, boards, fans, pool, IP, MAC, firmware, and model when the firmware exposes those fields.

The dashboard also tracks last checked and last seen times. Live monitoring refreshes the selected miner detail popup automatically when that selected IP is seen again.

The rack map header includes fleet summary chips for present, VNISH, Bitmain, auth-required, web-only, SSH-only, offline, and unknown slots.

Edit mode is the only mode that arms a square for IP Report assignment.

## Existing behavior kept

- UDP IP Report listener.
- VNISH + Bitmain/Stock/Hiveon auto apply.
- Safe apply order.
- Parking IPs.
- Skip already-correct rows.
- Wrong-subnet report guard.

## Cleanup note

The cleaned package keeps only the launch GIF, app icon, header logo, source code, build files, and current README notes. Older generated splash frame PNGs and source MP4 files are not included.
