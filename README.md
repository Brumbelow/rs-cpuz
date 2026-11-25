# rs-cpuz

System profiler for Windows, written in Rust. Tabbed window, text and HTML reports.

- One executable, about 2 MB. No installer, no runtime dependencies.
- Runs without administrator rights. Asks for them at start to read more (see [Administrator rights and PawnIO](#administrator-rights-and-pawnio)).
- Reports also list drives, volumes, drive health, USB devices, monitors and sensors.
- Benchmark and stress test.
- Command-line reports, tab snapshots and benchmark runs.
- Dark UI, per-monitor DPI aware.

## Requirements

- Windows 10 1709 or later, or Windows 11. x64 (an x86 build is possible).
- Optional: the [PawnIO](https://pawnio.eu) driver, for ring-0 data.

## Tabs

| Tab | Contents |
| --- | --- |
| CPU | Name, code name, package, process node, TDP, family/model/stepping, revision, instruction sets, core speed, multiplier, bus speed, caches (per core type on hybrid CPUs), core and thread counts. With PawnIO: core voltage, multiplier range |
| Mainboard | Board vendor and model, bus specification, chipset, southbridge, BIOS brand/version/date, microcode, graphic interface link width and speed |
| Memory | Type, size, channels, DRAM frequency, FSB:DRAM. With PawnIO: CL, tRCD, tRP, tRAS, tRC, uncore frequency |
| SPD | Per slot: size, type, bandwidth, manufacturer, part and serial number. With PawnIO: JEDEC timings |
| Graphics | Adapter, board vendor, code name, revision, process node, clocks, memory size and type, TMUs |
| Bench | Single- and multi-thread benchmark, stress test, saved reference scores |
| About | Version, Windows build, DirectX level, access level, count of parts that could not be read |

## Tools menu

| Entry | Function |
| --- | --- |
| Save Report (.TXT) / (.HTML) | Write a report |
| Clocks | Frequency and multiplier of every core |
| Timers | System time sources side by side |
| Sensors | Power, temperature, fan and voltage readings, updated every second |
| Restart as Administrator | Shown when not elevated, except with `-demo` |

## Administrator rights and PawnIO

Elevation:

- At window start, an administrator running with a filtered token gets a UAC prompt. Standard users are not prompted.
- Declined prompt: the program continues without administrator rights.
- No prompt with `-noadmin`, `Elevate=0`, `-demo`, or any command-line report mode.

PawnIO is used when all of these hold:

- the driver is installed (from [pawnio.eu](https://pawnio.eu));
- the program runs as administrator;
- `PawnIO=1` (default).

Access is read-only. rs-cpuz has no code path that writes an MSR, a memory-controller register or an SMBus device.

| Data | Processors |
| --- | --- |
| Core voltage (VID), package and per-core temperatures, multiplier range | Intel Core, Sandy Bridge to Arrow Lake |
| Uncore (ring) frequency | Intel Core, Skylake to Raptor Lake |
| Memory timings, exact channel count | Intel 12th to 14th Gen desktop and HX (Alder Lake-S, Raptor Lake-S) |
| SPD JEDEC timings | Intel chipsets (i801 SMBus). DDR5: only the SPD page the firmware left selected |

AMD processors: nothing is read through PawnIO.

SMBus reads hold the `Global\Access_SMBUS.HTP.Method` mutex, shared with other monitoring tools.

The About tab and the reports show the status:

| Text | Meaning |
| --- | --- |
| `Administrator: yes` / `no` | Process elevation |
| `PawnIO: <version>` | Driver in use |
| `PawnIO: not installed` | No driver |
| `PawnIO: needs administrator` | Driver installed, process not elevated |
| `PawnIO: disabled` | `PawnIO=0` |
| `PawnIO: <version> installed, not running` | Device does not open |
| `PawnIO: <version> (nothing to read on this processor)` | Processor not supported |

The PawnIO modules (`IntelMSR`, `IntelMCHBAR`, `SmbusI801`, PawnIO.Modules 0.2.12) are embedded in the executable. License LGPL-2.1-or-later. Hashes and source: [assets/pawnio](assets/pawnio/README.md).

## Building

Rust 1.81 or newer.

```bash
cargo build --release
cargo test
```

Output: `target/release/rs-cpuz.exe`. The icon, version information and manifest are linked in by the build script.

| Toolchain | Needs |
| --- | --- |
| MSVC (`x86_64-pc-windows-msvc`) | Visual Studio build tools (`rc.exe`). The C runtime is linked statically through `.cargo/config.toml`, which Cargo reads from the current directory: build from the repository root. |
| GNU (`x86_64-pc-windows-gnu`) | MinGW-w64 with `gcc` and `windres` on `PATH`. Also builds from Linux with `--target x86_64-pc-windows-gnu`. |

## Command line

No option opens the window. Options start with `-`, `--` or `/` and are not case sensitive.

| Option | Effect |
| --- | --- |
| `-txt=NAME` | Write the text report to `NAME.txt` and exit |
| `-html=NAME` | Write the HTML report to `NAME.htm` and exit |
| `-bench` | Run the benchmark without a window, save the scores to `<computer name>.txt` |
| `-core=N` | Show the clocks of core `N` on the CPU tab |
| `-snap=DIR` | Save every tab as a `.bmp` file in `DIR` and exit |
| `-dump` | Print the detected hardware model and exit |
| `-demo` | Use a built-in sample system instead of the hardware |
| `-noadmin` | Do not ask for administrator rights at window start |
| `-help` | Show the option list |

A relative `NAME` or `DIR` is resolved next to the executable. Benchmark scores and saved references go to the executable's folder, or to `%APPDATA%\rs-cpuz` when that folder is read-only (for example under Program Files).

`-help`, `-dump` and errors print to the console that started rs-cpuz. It is a windowed program, so an interactive shell does not wait for it and the text can appear after the next prompt. Redirected or piped output is unaffected.

## Keyboard

| Key | Action |
| --- | --- |
| `F5` | Save the current tab as `.bmp` |
| `F6` | Copy the current tab to the clipboard |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Up` / `Down` / `PgUp` / `PgDn` / `Home` / `End` | Move through an open list, scroll the Clocks table |
| `Esc` | Close a list or tool window |

Right-click the Clocks group on the CPU tab to choose the core it shows.

## Configuration

Optional `rs-cpuz.ini`, read from the executable's folder, then `%APPDATA%\rs-cpuz`.

```ini
[RS-CPUZ]
TextFontName=
TextFontSize=14
TextFontColor=FFFFFF
LabelFontName=
LabelFontSize=14
DMI=1
PCI=1
Display=1
Elevate=1
PawnIO=1
```

| Key | Default | Effect |
| --- | --- | --- |
| `TextFontName`, `TextFontSize`, `TextFontColor` | built-in, 14, theme | Font of values in boxes and lists. Colour is `RRGGBB` hex |
| `LabelFontName`, `LabelFontSize` | built-in, 14 | Font of labels, buttons and tab titles |
| `DMI` | 1 | `0`: skip the SMBIOS tables |
| `PCI` | 1 | `0`: skip PCI devices. Reports then list drives without their controllers |
| `Display` | 1 | `0`: skip display adapters. Reports then omit monitors |
| `Elevate` | 1 | `0`: no UAC prompt at window start |
| `PawnIO` | 1 | `0`: never use the PawnIO driver |

Flags accept `1`/`0`, `on`/`off`, `true`/`false`, `yes`/`no`. Any text encoding Notepad saves is read.

## Data sources

| Data | Source |
| --- | --- |
| Processor identification, caches, features | `CPUID`, once per logical processor |
| Topology, hybrid core types | `GetLogicalProcessorInformationEx` |
| Preferred cores | `GetSystemCpuSetInformation` scheduling classes |
| Core speed | Processor performance counters, calibrated time-stamp counter |
| Mainboard, BIOS, memory modules | SMBIOS (`GetSystemFirmwareTable`) |
| Chipset, PCI devices, link width and speed | SetupAPI device properties |
| Graphics adapters | DXGI, `D3DKMTQueryAdapterInfo`, display class registry keys, NVAPI when present |
| GPU temperature, fan, clocks | D3DKMT adapter performance data, NVAPI |
| CPU power | `\Energy Meter(*)` performance counters (RAPL) |
| ACPI temperatures | `\Thermal Zone Information(*)` performance counters |
| Monitors | EDID stored by Windows |
| Drives and volumes | `IOCTL_STORAGE_QUERY_PROPERTY` on disk handles opened without read or write access, `GetDiskFreeSpaceEx` |
| Drive health and temperature | NVMe health log, ATA device statistics log (04h). Not queried while a disk is spun down |
| USB devices | SetupAPI, `IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX` |
| Hardware error count | WHEA events in the System event log |
| Ring-0 data | PawnIO (optional, see above) |

## Limitations

- Without PawnIO and administrator rights, these are blank: core voltage, CPU temperatures (the ACPI zone is still read), memory timings, SPD timings, uncore frequency, minimum multiplier.
- Not read: Super I/O (LPCIO) chip and its fans and voltages; XMP and EXPO profiles; the DDR5 SPD manufacturing page (needs a page-select write); command rate; tRFC; rated FSB; register dumps.
- Core speed is the effective frequency while the core was busy. An idle core reads lower than tools that latch the P-state from an MSR.
- The uncore frequency is the ratio times 100 MHz, not times the measured base clock.
- FSB:DRAM assumes a 100 MHz base clock. Blank when the firmware reports another bus clock.
- Benchmark scores use rs-cpuz's own workload and compare only within the same benchmark version.
  - Save as Reference stores a result under the processor name. A run on fewer threads than the processor has is stored separately, with the thread count in the name.
  - `-demo` still benchmarks the real processor. `-bench -demo` writes this machine's scores under the sample system's name.
- Code names, process nodes, packages and TDP come from tables in this repository. A missing part shows blank fields.

## Disclaimer

Independent project. Not affiliated with or endorsed by CPUID (CPU-Z), MSI or any other vendor. Contains none of their code or artwork. Product names and trademarks belong to their owners.

## License

[Apache License 2.0](LICENSE-APACHE). Embedded PawnIO modules: LGPL-2.1-or-later ([assets/pawnio/COPYING](assets/pawnio/COPYING)).
