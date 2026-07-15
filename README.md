# BlockOps Static IP Map Tool

A high-speed IP assignment and network mapping tool built specifically for large ASIC mining operations.

Designed for technicians working hundreds or thousands of miners, drastically reduces the time required to assign static IP addresses, remap racks, and recover from network changes.

---

## Features

✅ Sequential Static IP Assignment

- Automatically assigns target IPs in order
- Eliminates manual typing
- Prevents duplicate assignments

---

✅ Rack Mapping

- Visual rack layout
- Click any slot to arm it
- Press the miner's physical **IP Report** button
- The miner is automatically captured into the selected rack position

---

✅ Safe IP Migration

Moves miners without causing IP collisions.

The application automatically:

- Builds dependency chains
- Uses temporary parking IPs
- Executes migrations in the correct order
- Supports configurable parallel execution

---

✅ Mixed Firmware Support

Automatically detects and configures:

- VNISH
- Bitmain Stock Firmware
- Hiveon Firmware

No firmware selection required.

---

✅ Monitoring

- Rack status visualization
- Online/offline monitoring
- Parallel network scans
- Auto refresh support

---

## Designed For

This application was built for:

- Large mining farms
- Hosting facilities
- Repair technicians
- Deployment teams
- Rack rebuilds
- Mass firmware changes


---

## Typical Workflow

1. Enter starting IP
2. Click **Start Listening**
3. Press each miner's IP Report button
4. Verify the rack map
5. Build Safe Apply Order
6. Apply Changes

Done.

---

## Features Included

- Sequential IP Assignment
- Rack Mapping
- Safe Apply Engine
- Parking IP Support
- Redo Individual Rows
- Skip Rows
- Undo Last Scan
- Delete Rows
- Parallel Apply
- Export Results
- Copy Failed Results
- CSV Logging
- Auto Save Results
- Windows GUI
- Embedded Splash Screen

---

## Building

```bash
build_release.bat
```

Output:

```
dist/BlockOps_Static_IP_Tool.exe
```

---

## Requirements

- Windows
- Rust (for building)
- Network access to miners

---

## Supported Firmware

| Firmware | Supported |
|-----------|-----------|
| VNISH | ✅ |
| Bitmain Stock | ✅ |
| Hiveon | ✅ |

---

