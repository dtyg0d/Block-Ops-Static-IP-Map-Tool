# BlockOps Static IP Tool - Splash + Toggle Version

Changes in this build:

- Added first-pass Rack Map Monitor workflow:
  - Click a rack/slot square to arm its target IP.
  - Press the physical miner IP Report button to capture that miner into the selected slot.
  - Optional auto-apply uses the existing safe apply flow immediately.
  - Manual monitor scan paints expected slot IPs online/offline.
  - Rack map now stacks vertically and monitor scans run in parallel.
- Your startup animation is embedded as PNG frames inside the Rust binary.
- The listener button is now a toggle:
  - Start Listening
  - Stop Listening
- Sequential target IP assignment is still included.
- Skip Next IP is still included.
- Safe apply order with parking IPs 168-240 is still included.

## Build

Run:

```text
build_release.bat
```

Final EXE:

```text
dist\BlockOps_Static_IP_Tool.exe
```

## Notes

Embedded splash frames: 40

The listener stop button signals the listener thread to stop. It may take up to half a second to fully release the UDP port.


## v0.5 Changes

- Miner list now has more vertical room.
- Miner list auto-scrolls to the newest scanned miner by default.
- Added **Auto-scroll** checkbox.
- Added **Delete Selected**:
  - Click any cell in a miner row.
  - Click Delete Selected.
  - The row is removed.
  - Its target IP is released back into the run.
  - Safe apply order is cleared so it can be rebuilt.
- Added **Undo Last** for quickly removing the most recent scan if you accidentally pressed the wrong IP Report button.


## v0.6 Changes - BTC-style row redo

- Clicking any miner row now arms that row's target IP for redo.
- The next IP Report overwrites that row's Current IP and MAC.
- After the overwrite, the app automatically clears redo mode.
- The normal sequential Next Target IP is preserved, so if you are on `.24`, redo `.3`, then the next normal report continues at `.24`.
- Added **Cancel Redo**.
- Duplicate current IP/MAC checks prevent accidentally assigning the same reporting miner to two different rows.

## v0.7 Audit Fixes

I manually reviewed v0.6 and found two compile-risk issues plus a few workflow bugs:

- Added missing `select_row_for_redo` and `cancel_redo` functions.
- Fixed a Rust borrow-checker issue in the table click handler by collecting the clicked line first, then mutating state after the row loop.
- Added configurable **Step Delay** between safe-apply steps. Default is 8 seconds so parking moves have time to come back online before the final move.
- Skip now clears the previously-built safe order so stale apply plans don't get used.
- Delete Selected / Undo Last now rewind the next target IP when deleting the latest sequential row.

Note: I could not run `cargo check` in ChatGPT's container because Cargo is not installed there, so this was a static code audit and patch.


## v0.8 Changes

- Release EXE no longer opens a command prompt window when launched directly.
  - This uses the Windows GUI subsystem in release builds.
  - Building with `build_release.bat` will still show a command window while building; the finished EXE should not.
- Window now opens roughly BTC-tool sized.
- On Windows, the app centers itself on the primary monitor after launch.


## v0.8.1 Build Fix

- Fixed Windows HWND pointer comparison compile error:
  - `hwnd == 0`
  - changed to `hwnd == std::ptr::null_mut()`

The RetainedImage messages are warnings only and do not block the build.


## v0.9 Changes - Apply Result Copy/Export

- Added **Export Apply Results** button.
- Added **Copy Results** button.
- Added **Copy Failed Only** button.
- Safe apply/results table is taller.
- Shows a failed-row count beside the safe apply table.
- Apply step status changes are also written into the log file.


## v1.0 Results Layout Fix

- Full page can scroll vertically now, so lower sections should not get trapped or overlap.
- Apply results table has more space.
- Long error messages are shortened on screen but still visible on hover.
- Every apply status update auto-saves to:
  `blockops_apply_results_YYYYMMDD_HHMMSS.csv`
- The log still records apply status updates.
- Window opens larger to make rack runs easier to review.


## v1.1 Changes

### Skip rows
- **Skip Next IP** now creates a normal row in the main scan table.
- The row shows `SKIPPED -> target IP`.
- Click the skipped row later, then press IP Report to overwrite/fill that target IP.
- Skipped/unfilled rows are ignored by Safe Apply until they are filled with a real current IP.

### Faster apply
- Added **Parallel Jobs**.
- Safe Apply now runs independent steps in the same safe wave concurrently.
- Dependency waves still run in order so conflict handling and parking still work.
- **Step Delay** now waits between safe waves instead of after every single miner.
- Default is now 12 parallel jobs and 2 seconds between waves.

### Firmware modes
- Added firmware selector:
  - **VNISH API** uses `/api/v1/unlock` + `/api/v1/settings`
  - **Stock/Hiveon CGI** uses `/cgi-bin/set_network_conf.cgi`
- Stock/Hiveon CGI mode sends the form fields used by the uploaded `set_network_conf.cgi`.
- Added `User` field for CGI digest/basic auth, default `root`.
- Password field is shared:
  - VNISH: miner password
  - Stock/Hiveon CGI: HTTP auth password

### Notes
- For Stock/Hiveon CGI mode, DNS2 is ignored because the CGI script only validates one DNS server.
- If the miner restarts networking and closes the HTTP connection, CGI mode treats common connection-reset messages as sent/successful.


## v1.2 Changes

### Separate auth defaults
- VNISH mode uses only **VNISH Pwd**, default `admin`.
- Stock/Hiveon CGI mode uses:
  - User: `root`
  - Password: `root`
- The UI only shows the auth fields relevant to the selected firmware mode.

### One main table
- Removed the large separate Safe Apply / Apply Results table from the bottom.
- Main table now includes:
  - Scan Status
  - Apply Order
  - Wave
  - Type
  - Apply Result
- Build Safe Apply Order fills the Apply Order/Wave/Type columns.
- Apply Safe Order updates Apply Result directly in the main table.
- Skipped rows stay in the main table and can be clicked later to redo/fill that target IP.


## v1.3 Bitmain Stock Endpoint Fix

Confirmed against Bitmain's published Antminer API docs:

- Endpoint: `/cgi-bin/set_network_conf.cgi`
- Auth: digest HTTP auth, normally `root:root`
- Newer stock Bitmain payload:
  ```json
  {
    "ipHost": "miner-10-5-10-97",
    "ipPro": 2,
    "ipAddress": "10.5.10.97",
    "ipSub": "255.255.255.0",
    "ipGateway": "10.5.10.254",
    "ipDns": "1.1.1.1"
  }
  ```

The app now does this in **Bitmain Stock/Hiveon** mode:

1. Try the newer official Bitmain JSON payload first.
2. If that fails, fall back to the older `_ant_conf_*` form payload used by legacy Bitmain/Hiveon-style CGI scripts.
3. Uses at least a 15 second timeout for stock CGI apply because some stock scripts restart networking/sleep before returning.


## v1.4 Auto All-In-One Firmware Mode

- Removed the manual firmware selector.
- Every apply step now runs in **Auto VNISH + Bitmain Stock/Hiveon** mode:
  1. Try VNISH API first using VNISH password, default `admin`.
  2. If VNISH fails, automatically fall back to Bitmain Stock/Hiveon CGI using `root:root`.
  3. Stock/Hiveon first tries the newer Bitmain JSON payload, then falls back to the legacy `_ant_conf_*` form payload.
- This lets one rack sweep contain mixed VNISH, stock Bitmain, and Hiveon-style firmware without changing modes.
- VNISH auto probe uses a shorter timeout, 3-5 seconds, so stock miners do not waste as much time failing the VNISH API before fallback.


## v1.4.1 Compile Fix

- Fixed Rust compile error:
  - Removed unsupported `ureq::Request.auth(...)`
  - Replaced it with manual `Authorization: Basic ...` header support.
- Digest auth path is unchanged.
- The `RetainedImage` messages are warnings only and do not block the build.
