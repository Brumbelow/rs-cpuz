//! Names for PCI class codes (public PCI-SIG class code assignments).
//!
//! A device states what it is with three bytes: base class, sub-class and
//! programming interface. The name follows the sub-class, narrowed by the
//! programming interface where that picks one well-known kind of controller.

/// Name of the kind of device a class code stands for, e.g. `0x06, 0x00` ->
/// `Host Bridge`. `None` for codes without an assignment and for base class 0,
/// which only says that the device predates the class codes.
pub fn class_name(base: u8, sub: u8, prog_if: u8) -> Option<&'static str> {
    let name = match (base, sub) {
        (0x01, 0x00) => "SCSI Controller",
        (0x01, 0x01) => "IDE Controller",
        (0x01, 0x02) => "Floppy Disk Controller",
        (0x01, 0x03) => "IPI Bus Controller",
        (0x01, 0x04) => "RAID Controller",
        (0x01, 0x05) => "ATA Controller",
        (0x01, 0x06) => match prog_if {
            0x01 => "Serial ATA Controller - AHCI",
            _ => "Serial ATA Controller",
        },
        (0x01, 0x07) => "Serial Attached SCSI (SAS) Controller",
        (0x01, 0x08) => match prog_if {
            0x02 | 0x03 => "NVM Express (NVMe) Controller",
            _ => "Non-Volatile Memory Controller",
        },
        (0x01, 0x09) => "Universal Flash Storage (UFS) Controller",
        (0x01, 0x80) => "Mass Storage Controller",

        (0x02, 0x00) => "Ethernet Controller",
        (0x02, 0x01) => "Token Ring Controller",
        (0x02, 0x02) => "FDDI Controller",
        (0x02, 0x03) => "ATM Controller",
        (0x02, 0x04) => "ISDN Controller",
        (0x02, 0x05) => "WorldFip Controller",
        (0x02, 0x06) => "PICMG Multi Computing Controller",
        (0x02, 0x07) => "InfiniBand Controller",
        (0x02, 0x08) => "Host Fabric Controller",
        (0x02, 0x80) => "Network Controller",

        (0x03, 0x00) => match prog_if {
            0x01 => "8514 Controller",
            _ => "VGA Controller",
        },
        (0x03, 0x01) => "XGA Controller",
        (0x03, 0x02) => "3D Controller",
        (0x03, 0x80) => "Display Controller",

        (0x04, 0x00) => "Video Device",
        (0x04, 0x01) => "Audio Device",
        (0x04, 0x02) => "Computer Telephony Device",
        (0x04, 0x03) => "High Definition Audio (HDA)",
        (0x04, 0x80) => "Multimedia Device",

        (0x05, 0x00) => "RAM Controller",
        (0x05, 0x01) => "Flash Memory Controller",
        (0x05, 0x02) => "CXL Memory Device",
        (0x05, 0x80) => "Memory Controller",

        (0x06, 0x00) => "Host Bridge",
        (0x06, 0x01) => "PCI to ISA Bridge",
        (0x06, 0x02) => "PCI to EISA Bridge",
        (0x06, 0x03) => "PCI to MCA Bridge",
        (0x06, 0x04) => "PCI to PCI Bridge",
        (0x06, 0x05) => "PCI to PCMCIA Bridge",
        (0x06, 0x06) => "PCI to NuBus Bridge",
        (0x06, 0x07) => "PCI to CardBus Bridge",
        (0x06, 0x08) => "PCI to RACEway Bridge",
        (0x06, 0x09) => "Semi-Transparent PCI to PCI Bridge",
        (0x06, 0x0A) => "InfiniBand to PCI Host Bridge",
        (0x06, 0x0B) => "Advanced Switching to PCI Host Bridge",
        (0x06, 0x80) => "Bridge Device",

        (0x07, 0x00) => "Serial Port",
        (0x07, 0x01) => "Parallel Port",
        (0x07, 0x02) => "Multiport Serial Controller",
        (0x07, 0x03) => "Modem",
        (0x07, 0x04) => "GPIB Controller",
        (0x07, 0x05) => "Smart Card Controller",
        (0x07, 0x80) => "Communication Device",

        (0x08, 0x00) => match prog_if {
            0x10 | 0x20 => "I/O APIC",
            _ => "Interrupt Controller",
        },
        (0x08, 0x01) => "DMA Controller",
        (0x08, 0x02) => match prog_if {
            0x03 => "High Precision Event Timer (HPET)",
            _ => "System Timer",
        },
        (0x08, 0x03) => "Real Time Clock",
        (0x08, 0x04) => "PCI Hot-Plug Controller",
        (0x08, 0x05) => "SD Host Controller",
        (0x08, 0x06) => "IOMMU",
        (0x08, 0x07) => "Root Complex Event Collector",
        (0x08, 0x80) => "System Peripheral",

        (0x09, 0x00) => "Keyboard Controller",
        (0x09, 0x01) => "Digitizer",
        (0x09, 0x02) => "Mouse Controller",
        (0x09, 0x03) => "Scanner Controller",
        (0x09, 0x04) => "Gameport Controller",
        (0x09, 0x80) => "Input Device",

        (0x0A, 0x00 | 0x80) => "Docking Station",

        (0x0B, 0x00) => "386 Processor",
        (0x0B, 0x01) => "486 Processor",
        (0x0B, 0x02) => "Pentium Processor",
        (0x0B, 0x10) => "Alpha Processor",
        (0x0B, 0x20) => "PowerPC Processor",
        (0x0B, 0x30) => "MIPS Processor",
        (0x0B, 0x40) => "Co-Processor",
        (0x0B, 0x80) => "Processor",

        (0x0C, 0x00) => "IEEE 1394 Controller",
        (0x0C, 0x01) => "ACCESS.bus Controller",
        (0x0C, 0x02) => "SSA Controller",
        (0x0C, 0x03) => match prog_if {
            0x00 => "USB Controller (UHCI)",
            0x10 => "USB Controller (OHCI)",
            0x20 => "USB Controller (EHCI)",
            0x30 => "USB Controller (xHCI)",
            0x40 => "USB4 Host Interface",
            0xFE => "USB Device",
            _ => "USB Controller",
        },
        (0x0C, 0x04) => "Fibre Channel Controller",
        (0x0C, 0x05) => "SMBus Controller",
        (0x0C, 0x06) => "InfiniBand Controller",
        (0x0C, 0x07) => "IPMI Interface",
        (0x0C, 0x08) => "SERCOS Interface",
        (0x0C, 0x09) => "CAN Bus Controller",
        (0x0C, 0x0A) => "MIPI I3C Controller",
        (0x0C, 0x80) => "Serial Bus Controller",

        (0x0D, 0x00) => "IrDA Controller",
        (0x0D, 0x01) => "Consumer IR Controller",
        (0x0D, 0x10) => "RF Controller",
        (0x0D, 0x11) => "Bluetooth Controller",
        (0x0D, 0x12) => "Broadband Controller",
        (0x0D, 0x20) => "802.11a Controller",
        (0x0D, 0x21) => "802.11b Controller",
        (0x0D, 0x40) => "Cellular Modem",
        (0x0D, 0x41) => "Cellular Modem with Ethernet",
        (0x0D, 0x80) => "Wireless Controller",

        (0x0E, 0x00) => "I2O Controller",

        (0x0F, 0x01) => "Satellite TV Controller",
        (0x0F, 0x02) => "Satellite Audio Controller",
        (0x0F, 0x03) => "Satellite Voice Controller",
        (0x0F, 0x04) => "Satellite Data Controller",

        (0x10, 0x00) => "Network Encryption Device",
        (0x10, 0x10) => "Entertainment Encryption Device",
        (0x10, 0x80) => "Encryption Device",

        (0x11, 0x00) => "DPIO Module",
        (0x11, 0x01) => "Performance Counters",
        (0x11, 0x10) => "Communication Synchronizer",
        (0x11, 0x20) => "Management Card",
        (0x11, 0x80) => "Signal Processing Controller",

        (0x12, 0x00) => "Processing Accelerator",
        (0x13, 0x00) => "Non-Essential Instrumentation",
        (0x40, 0x00) => "Co-Processor",
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_classes_of_a_desktop() {
        assert_eq!(class_name(0x06, 0x00, 0x00), Some("Host Bridge"));
        assert_eq!(class_name(0x06, 0x01, 0x00), Some("PCI to ISA Bridge"));
        assert_eq!(class_name(0x06, 0x04, 0x00), Some("PCI to PCI Bridge"));
        assert_eq!(class_name(0x02, 0x00, 0x00), Some("Ethernet Controller"));
        assert_eq!(class_name(0x02, 0x80, 0x00), Some("Network Controller"));
        assert_eq!(class_name(0x03, 0x00, 0x00), Some("VGA Controller"));
        assert_eq!(class_name(0x04, 0x03, 0x00), Some("High Definition Audio (HDA)"));
        assert_eq!(class_name(0x0C, 0x05, 0x00), Some("SMBus Controller"));
        assert_eq!(class_name(0x10, 0x80, 0x00), Some("Encryption Device"));
    }

    #[test]
    fn programming_interface_narrows_the_name() {
        assert_eq!(class_name(0x01, 0x06, 0x00), Some("Serial ATA Controller"));
        assert_eq!(class_name(0x01, 0x06, 0x01), Some("Serial ATA Controller - AHCI"));
        assert_eq!(class_name(0x01, 0x08, 0x02), Some("NVM Express (NVMe) Controller"));
        assert_eq!(class_name(0x01, 0x08, 0x01), Some("Non-Volatile Memory Controller"));
        assert_eq!(class_name(0x0C, 0x03, 0x00), Some("USB Controller (UHCI)"));
        assert_eq!(class_name(0x0C, 0x03, 0x20), Some("USB Controller (EHCI)"));
        assert_eq!(class_name(0x0C, 0x03, 0x30), Some("USB Controller (xHCI)"));
        assert_eq!(class_name(0x0C, 0x03, 0x80), Some("USB Controller"));
        assert_eq!(class_name(0x08, 0x02, 0x03), Some("High Precision Event Timer (HPET)"));
        assert_eq!(class_name(0x03, 0x00, 0x01), Some("8514 Controller"));
        // A subtractive-decode bridge is a PCI to PCI bridge all the same.
        assert_eq!(class_name(0x06, 0x04, 0x01), Some("PCI to PCI Bridge"));
    }

    #[test]
    fn unassigned_codes_have_no_name() {
        assert_eq!(class_name(0x00, 0x00, 0x00), None);
        assert_eq!(class_name(0x00, 0x01, 0x00), None);
        assert_eq!(class_name(0x06, 0x0C, 0x00), None);
        assert_eq!(class_name(0x0E, 0x01, 0x00), None);
        assert_eq!(class_name(0x14, 0x00, 0x00), None);
        assert_eq!(class_name(0x41, 0x00, 0x00), None);
        assert_eq!(class_name(0xFF, 0x00, 0x00), None);
        assert_eq!(class_name(0xFF, 0xFF, 0xFF), None);
    }

    #[test]
    fn every_name_is_a_single_printable_line() {
        for base in 0..=u8::MAX {
            for sub in 0..=u8::MAX {
                for prog_if in [0x00, 0x01, 0x02, 0x03, 0x10, 0x20, 0x30, 0x40, 0x80, 0xFE, 0xFF] {
                    if let Some(name) = class_name(base, sub, prog_if) {
                        assert!(!name.is_empty() && name.chars().all(|c| c.is_ascii_graphic() || c == ' '));
                        assert_eq!(name, name.trim());
                    }
                }
            }
        }
    }
}
