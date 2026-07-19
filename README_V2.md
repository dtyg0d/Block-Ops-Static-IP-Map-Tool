# BlockOps Static IP Manager 2.1.1

This edition uses a commercial operations-console UI and keeps rack monitoring, assignment, and site configuration in focused workspaces.

## Rack Dashboard

- Scan all racks or type one rack number to scan only that rack.
- Start and stop continuous monitoring.
- Click slots to view miner details.
- Switch to `Assign IP` mode to arm a physical rack slot for IP Report.
- Uses the 19-rack map from `10.4.1.1-168` through `10.4.19.1-168`.

## IP Assignment

- Start or stop the IP Report listener.
- Set the first target IP and capture miners in physical order.
- Review the assignment queue without accidentally arming redo.
- Use the explicit `Redo selected` action when a selected row must be replaced.
- Pre-check, apply in safe order, and export plan or result files.

## Settings

- Rack layout and address base.
- Monitoring interval, parallel jobs, timeout, and wave delay.
- IP Report port and wrong-subnet protection.
- VNISH and Bitmain credentials.
- Netmask, gateway, and DNS values.

## Reliability changes

- Background results repaint automatically while scans and applies run.
- Apply batches cannot overlap; later reports queue for the next batch.
- Successful apply steps immediately update the stored current IP.
- Known VNISH and Bitmain miners use the matching setter directly.
- A known-online miner requires two failed checks before being marked offline.
- Stop Live cancels between probes and ignores late results.

## Interface changes

- Neutral graphite product theme with native Windows typography.
- Exact two-rack layout without horizontal scrolling.
- Responsive assignment controls and a structured selectable queue.
- Unified status indicators, miner details, settings, and safety dialogs.
- Fleet counters now live in the workspace header so a complete 168-slot rack fits onscreen.

Run `build_release.bat` to create the Windows release executable.
