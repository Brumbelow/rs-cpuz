//! Memory channel inference from the firmware's slot labels.
//!
//! Which channel a slot hangs off is a property of the memory controller, and its
//! registers are out of reach from user mode. Most firmware names the channel in
//! the SMBIOS device or bank locator though (`ChannelA-DIMM0`, `P0 CHANNEL B`,
//! `DIMM_A2`), so the populated channels can be counted from the labels of the
//! installed modules. Labels that say nothing about channels (`DIMM 1`, `BANK 0`)
//! give no answer rather than a guess.
//!
//! The count is per processor, the way a memory controller is described: a
//! two-socket board with four channels behind each processor is quad channel.

use std::collections::BTreeSet;

use super::dmi::MemoryType;

/// Words that introduce a processor number: `P0 CHANNEL A`, `CPU1_DIMM_A1`.
const SOCKET_WORDS: &[&str] = &["CPU", "P", "PROC", "PROCESSOR", "SOCKET"];

/// Words that introduce a NUMA node number: `P0_Node1_Channel0_Dimm0`.
const NODE_WORDS: &[&str] = &["NODE"];

/// Words that introduce a memory controller number: `Controller1-ChannelA-DIMM0`.
const CONTROLLER_WORDS: &[&str] = &["CONTROLLER"];

/// A piece of a locator: a run of letters (upper-cased) or a run of digits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Number(u32),
}

/// The tokens of both labels of one slot.
struct Labels {
    device: Vec<Token>,
    bank: Vec<Token>,
}

/// A channel as far as the labels identify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ChannelId {
    socket: Option<u32>,
    node: Option<u32>,
    controller: Option<u32>,
    /// `A` -> 0, `B` -> 1 ...; numbered channels keep their number.
    channel: u32,
}

/// One way of reading a slot's channel out of its labels.
type Strategy = fn(&Labels) -> Option<ChannelId>;

/// In order of trust: an explicit `Channel` keyword in either label, then the
/// `DIMM<channel><slot>` naming scheme.
const STRATEGIES: [Strategy; 3] = [
    |labels| labels.identify(keyword_channel(&labels.device)),
    |labels| labels.identify(keyword_channel(&labels.bank)),
    |labels| labels.identify(dimm_letter_channel(&labels.device)),
];

impl Labels {
    fn new(device: &str, bank: &str) -> Self {
        Labels { device: tokens(device), bank: tokens(bank) }
    }

    /// Completes a channel with the socket, node and controller named by either label.
    fn identify(&self, channel: Option<u32>) -> Option<ChannelId> {
        let qualifier = |words| number_after(&self.device, words).or_else(|| number_after(&self.bank, words));
        Some(ChannelId {
            socket: qualifier(SOCKET_WORDS),
            node: qualifier(NODE_WORDS),
            controller: qualifier(CONTROLLER_WORDS),
            channel: channel?,
        })
    }
}

/// Splits a locator into letter runs and digit runs; anything else separates.
fn tokens(text: &str) -> Vec<Token> {
    let run = |text: &str, class: fn(&char) -> bool| text.find(|c: char| !class(&c)).unwrap_or(text.len());
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let length = if first.is_ascii_alphabetic() {
            let length = run(rest, char::is_ascii_alphabetic);
            tokens.push(Token::Word(rest[..length].to_ascii_uppercase()));
            length
        } else if first.is_ascii_digit() {
            let length = run(rest, char::is_ascii_digit);
            tokens.push(Token::Number(rest[..length].parse().unwrap_or(u32::MAX)));
            length
        } else {
            first.len_utf8()
        };
        rest = &rest[length..];
    }
    tokens
}

/// Zero-based index of a single channel letter.
fn letter_index(word: &str) -> Option<u32> {
    match word.as_bytes() {
        [letter @ b'A'..=b'Z'] => Some(u32::from(letter - b'A')),
        _ => None,
    }
}

/// The number that follows the first of `words`: `Controller1` -> 1, `P0` -> 0.
fn number_after(tokens: &[Token], words: &[&str]) -> Option<u32> {
    tokens.windows(2).find_map(|pair| match pair {
        [Token::Word(word), Token::Number(number)] if words.contains(&word.as_str()) => Some(*number),
        _ => None,
    })
}

/// The channel named by an explicit keyword: `ChannelA`, `CHANNEL B`, `Channel0`.
fn keyword_channel(tokens: &[Token]) -> Option<u32> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        let Token::Word(word) = token else { return None };
        match word.strip_prefix("CHANNEL")? {
            "" => match tokens.get(index + 1)? {
                Token::Word(next) => letter_index(next),
                Token::Number(number) => Some(*number),
            },
            suffix => letter_index(suffix),
        }
    })
}

/// Whether `word` introduces a processor, node or controller number.
fn is_qualifier(word: &str) -> bool {
    [SOCKET_WORDS, NODE_WORDS, CONTROLLER_WORDS].iter().any(|words| words.contains(&word))
}

/// The channel letter of a `<channel><slot>` pair that opens `tokens`.
fn letter_before_slot(tokens: &[Token]) -> Option<u32> {
    match tokens {
        [Token::Word(letter), Token::Number(_), ..] => letter_index(letter),
        _ => None,
    }
}

/// The channel letter of the `DIMM<channel><slot>` scheme: `DIMMA2`, `DIMM_B1`,
/// `SODIMM-C1`. The slot number is required, because a bare `DIMM A` is as often a
/// plain slot name as it is a channel.
///
/// Some boards put the processor in between, `DIMM_P0_B1`. Such a qualifier is
/// stepped over only when a channel and slot follow it: on its own `DIMM_P1` is
/// slot 1 of channel P.
fn dimm_letter_channel(tokens: &[Token]) -> Option<u32> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        let Token::Word(word) = token else { return None };
        let rest = &tokens[index + 1..];
        match word.rsplit_once("DIMM")?.1 {
            "" => match rest {
                [Token::Word(qualifier), Token::Number(_), after @ ..] if is_qualifier(qualifier) => {
                    letter_before_slot(after).or_else(|| letter_before_slot(rest))
                }
                _ => letter_before_slot(rest),
            },
            suffix => letter_index(suffix).filter(|_| matches!(rest, [Token::Number(_), ..])),
        }
    })
}

/// Number of channels behind each processor. `None` when the processors differ, when
/// only some labels name a processor, or when node numbers are the only qualifier:
/// those may count processors as well as the parts of one.
fn per_socket(channels: &BTreeSet<ChannelId>) -> Option<u32> {
    let sockets: BTreeSet<Option<u32>> = channels.iter().map(|id| id.socket).collect();
    let nodes: BTreeSet<Option<u32>> = channels.iter().map(|id| id.node).collect();
    if sockets.contains(&None) && (sockets.len() > 1 || nodes.len() > 1) {
        return None;
    }
    let mut counts = sockets.iter().map(|socket| channels.iter().filter(|id| id.socket == *socket).count());
    let first = counts.next()?;
    counts.all(|count| count == first).then_some(first).and_then(|count| u32::try_from(count).ok())
}

/// Counts the populated channels per processor from the `(device locator, bank
/// locator)` labels of the installed modules. `None` when there are no modules, when
/// the labels of any one of them do not name its channel, or when the count is
/// ambiguous.
pub fn populated<'a>(labels: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<u32> {
    let labels: Vec<Labels> = labels.into_iter().map(|(device, bank)| Labels::new(device, bank)).collect();
    if labels.is_empty() {
        return None;
    }
    let channels: BTreeSet<ChannelId> =
        STRATEGIES.iter().find_map(|strategy| labels.iter().map(strategy).collect::<Option<_>>())?;
    per_socket(&channels)
}

/// Counts the populated channels from the interleave positions the firmware gives the
/// installed modules. Only a set of distinct positions numbered 1 to N, one per module and
/// at least two, says that the modules sit on N channels interleaved with each other; a
/// missing, repeated or out-of-range position says nothing.
pub fn interleaved(positions: impl IntoIterator<Item = Option<u8>>) -> Option<u32> {
    let positions: Vec<u8> = positions.into_iter().collect::<Option<_>>()?;
    let distinct: BTreeSet<u8> = positions.iter().copied().collect();
    let count = positions.len();
    let numbered = distinct.len() == count && distinct.iter().copied().eq(1..=u8::try_from(count).ok()?);
    (count >= 2 && numbered).then(|| u32::try_from(count).ok()).flatten()
}

/// The "Channels #" text for `channels` populated channels. Controllers up to DDR4
/// are described by a channel-count name; DDR5 lists its 32-bit sub-channels, two
/// per channel. Counts without a name and LPDDR4/5 layouts are left unknown.
pub fn describe(kind: MemoryType, channels: u32) -> Option<String> {
    use MemoryType::*;
    match kind {
        Ddr5 if channels > 0 => Some(format!("{} x 32-bit", channels.saturating_mul(2))),
        Ddr | Ddr2 | Ddr2FbDimm | Ddr3 | Ddr4 | Lpddr3 => {
            let name = match channels {
                1 => "Single",
                2 => "Dual",
                3 => "Triple",
                4 => "Quad",
                5 => "Penta",
                6 => "Hexa",
                8 => "Octa",
                _ => return None,
            };
            Some(name.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> Token {
        Token::Word(text.to_string())
    }

    #[test]
    fn tokenizes_letters_and_digits() {
        assert_eq!(tokens("DIMMA2"), [word("DIMMA"), Token::Number(2)]);
        assert_eq!(tokens("Controller0-ChannelB"), [word("CONTROLLER"), Token::Number(0), word("CHANNELB")]);
        assert_eq!(tokens("P0 CHANNEL A"), [word("P"), Token::Number(0), word("CHANNEL"), word("A")]);
        assert_eq!(tokens(" dimm_b12 (left) "), [word("DIMM"), word("B"), Token::Number(12), word("LEFT")]);
        assert_eq!(tokens("Slot\u{00A0}\u{2116}3"), [word("SLOT"), Token::Number(3)]);
        assert_eq!(tokens("99999999999"), [Token::Number(u32::MAX)]);
        assert!(tokens("").is_empty() && tokens(" -_/ ").is_empty());
    }

    #[test]
    fn reads_channel_keywords() {
        let channel = |text: &str| keyword_channel(&tokens(text));
        assert_eq!(channel("ChannelA-DIMM0"), Some(0));
        assert_eq!(channel("ChannelB-DIMM1"), Some(1));
        assert_eq!(channel("P0 CHANNEL A"), Some(0));
        assert_eq!(channel("Controller0-ChannelB"), Some(1));
        assert_eq!(channel("CHANNEL-D"), Some(3));
        assert_eq!(channel("P0_Node0_Channel3_Dimm1"), Some(3));
        assert_eq!(channel("Channel 2"), Some(2));
        assert_eq!(channel("DIMMA2"), None);
        assert_eq!(channel("BANK 0"), None);
        assert_eq!(channel("CHANNEL"), None);
        assert_eq!(channel("Channel Left"), None);
        assert_eq!(channel(""), None);
    }

    #[test]
    fn reads_dimm_letter_scheme() {
        let channel = |text: &str| dimm_letter_channel(&tokens(text));
        assert_eq!(channel("DIMMA2"), Some(0));
        assert_eq!(channel("DIMM_B1"), Some(1));
        assert_eq!(channel("DIMM C1"), Some(2));
        assert_eq!(channel("P1-DIMMD1"), Some(3));
        assert_eq!(channel("CPU1_DIMM_A1"), Some(0));
        assert_eq!(channel("SODIMM-B1"), Some(1));
        assert_eq!(channel("sodimmb1"), Some(1));
        // The processor between `DIMM` and the channel.
        assert_eq!(channel("DIMM_P0_A0"), Some(0));
        assert_eq!(channel("DIMM_P0_B1"), Some(1));
        assert_eq!(channel("DIMM_P1_P0"), Some(15));
        assert_eq!(channel("DIMM_CPU0_C1"), Some(2));
        // Without a channel and slot behind it, `P1` is slot 1 of channel P.
        assert_eq!(channel("DIMM_P1"), Some(15));
        assert_eq!(channel("DIMMP1"), Some(15));
        assert_eq!(channel("DIMM_CPU1_2"), None);
        // No slot number: a slot name, not necessarily a channel.
        assert_eq!(channel("DIMM A"), None);
        assert_eq!(channel("DIMMB"), None);
        // Numbered slots carry no channel.
        assert_eq!(channel("DIMM1"), None);
        assert_eq!(channel("DIMM 0"), None);
        assert_eq!(channel("PROC 1 DIMM 4"), None);
        assert_eq!(channel("DIMM Slot 1"), None);
        assert_eq!(channel("ChannelA-DIMM0"), None);
        assert_eq!(channel("A1"), None);
        assert_eq!(channel("BANK A1"), None);
    }

    #[test]
    fn counts_channels_from_device_locators() {
        assert_eq!(populated([("DIMMA2", ""), ("DIMMB2", "")]), Some(2));
        assert_eq!(populated([("DIMM_A1", "BANK 0"), ("DIMM_A2", "BANK 1")]), Some(1));
        assert_eq!(populated([("DIMM_A1", "BANK 0"), ("DIMM_B1", "BANK 2")]), Some(2));
        assert_eq!(populated([("ChannelA-DIMM0", "BANK 0"), ("ChannelB-DIMM0", "BANK 2")]), Some(2));
        assert_eq!(populated([("ChannelA-DIMM0", "BANK 0"), ("ChannelA-DIMM1", "BANK 1")]), Some(1));
        let quad = ["DIMM_A1", "DIMM_B1", "DIMM_C1", "DIMM_D1", "DIMM_A2", "DIMM_B2", "DIMM_C2", "DIMM_D2"];
        assert_eq!(populated(quad.map(|locator| (locator, ""))), Some(4));
    }

    #[test]
    fn counts_channels_from_bank_locators() {
        assert_eq!(populated([("DIMM 0", "P0 CHANNEL A"), ("DIMM 0", "P0 CHANNEL B")]), Some(2));
        assert_eq!(populated([("DIMM 0", "P0 CHANNEL A"), ("DIMM 1", "P0 CHANNEL A")]), Some(1));
        assert_eq!(populated([("DIMMA2", "P0 CHANNEL A"), ("DIMMB2", "P0 CHANNEL B")]), Some(2));
        assert_eq!(populated([("SODIMM0", "CHANNEL A DIMM0"), ("SODIMM1", "CHANNEL B DIMM0")]), Some(2));
        assert_eq!(populated([("DIMM 1", "CHANNEL A")]), Some(1));
    }

    #[test]
    fn controllers_and_nodes_tell_equal_letters_apart() {
        let two_controllers =
            [("Controller0-ChannelA-DIMM0", "BANK 0"), ("Controller1-ChannelA-DIMM0", "BANK 0")];
        assert_eq!(populated(two_controllers), Some(2));
        let one_controller =
            [("Controller0-ChannelA-DIMM0", "BANK 0"), ("Controller0-ChannelA-DIMM1", "BANK 0")];
        assert_eq!(populated(one_controller), Some(1));
        assert_eq!(populated([("P0_Node0_Channel0_Dimm0", ""), ("P0_Node0_Channel0_Dimm1", "")]), Some(1));
        assert_eq!(populated([("P0_Node0_Channel0_Dimm0", ""), ("P0_Node1_Channel0_Dimm0", "")]), Some(2));
    }

    #[test]
    fn counts_channels_per_processor() {
        let balanced = ["P1-DIMMA1", "P1-DIMMB1", "P2-DIMMA1", "P2-DIMMB1"];
        assert_eq!(populated(balanced.map(|locator| (locator, ""))), Some(2));
        assert_eq!(populated([("DIMM 0", "P0 CHANNEL A"), ("DIMM 0", "P1 CHANNEL A")]), Some(1));
        // The processor may be named by the other label.
        let split = [("DIMMA1", "P0_Node0_Channel0_Dimm0"), ("DIMME1", "P1_Node1_Channel0_Dimm0")];
        assert_eq!(populated(split), Some(1));
        assert_eq!(populated([("DIMM_A1", "CPU 1"), ("DIMM_B1", "CPU 1"), ("DIMM_A1", "CPU 2")]), None);
        // Unevenly populated processors have no single channel count.
        assert_eq!(populated([("P1-DIMMA1", ""), ("P1-DIMMB1", ""), ("P2-DIMMA1", "")]), None);
        // Only some labels name a processor.
        assert_eq!(populated([("DIMM 0", "P0 CHANNEL A"), ("DIMM 1", "CHANNEL B")]), None);
        // Bare node numbers may be processors or halves of one.
        assert_eq!(populated([("DIMM_A1", "NODE 1"), ("DIMM_A1", "NODE 2")]), None);
        assert_eq!(populated([("DIMM_A1", "NODE 1"), ("DIMM_B1", "NODE 1")]), Some(2));
    }

    #[test]
    fn processor_between_dimm_and_channel_is_not_the_channel() {
        let one_socket = [
            ("DIMM_P0_A0", "BANK 0"),
            ("DIMM_P0_B0", "BANK 1"),
            ("DIMM_P0_C0", "BANK 2"),
            ("DIMM_P0_D0", "BANK 3"),
            ("DIMM_P0_E0", "BANK 4"),
            ("DIMM_P0_F0", "BANK 5"),
            ("DIMM_P0_G0", "BANK 6"),
            ("DIMM_P0_H0", "BANK 7"),
        ];
        assert_eq!(populated(one_socket), Some(8));
        assert_eq!(populated([("DIMM_P0_A0", ""), ("DIMM_P0_A1", "")]), Some(1));
        // The second processor carries on with the alphabet, up to its own letter.
        let second = ["I", "J", "K", "L", "M", "N", "O", "P"].map(|letter| format!("DIMM_P1_{letter}0"));
        let two_sockets = one_socket.into_iter().chain(second.iter().map(|locator| (locator.as_str(), "")));
        assert_eq!(populated(two_sockets), Some(8));
        assert_eq!(populated([("DIMM_P0_A0", ""), ("DIMM_P0_B0", ""), ("DIMM_P1_I0", "")]), None);
    }

    #[test]
    fn labels_without_channel_information_give_no_answer() {
        assert_eq!(populated([]), None);
        assert_eq!(populated([("DIMM1", ""), ("DIMM2", "")]), None);
        assert_eq!(populated([("DIMM 1", "BANK 0"), ("DIMM 2", "BANK 1")]), None);
        assert_eq!(populated([("PROC 1 DIMM 1", "Not Specified"), ("PROC 1 DIMM 2", "Not Specified")]), None);
        assert_eq!(populated([("A1", ""), ("B1", "")]), None);
        assert_eq!(populated([("DIMM A", ""), ("DIMM B", "")]), None);
        assert_eq!(populated([("M0001", ""), ("", "")]), None);
        // One module that cannot be placed spoils the count.
        assert_eq!(populated([("DIMMA1", ""), ("DIMM3", "")]), None);
    }

    #[test]
    fn one_strategy_covers_every_module() {
        // The first module names its channel in the bank label only, so the bank
        // label is used for both and the stray keyword in the second device label
        // does not split one channel into two.
        assert_eq!(populated([("DIMM 0", "P0 CHANNEL A"), ("ChannelB-DIMM1", "P0 CHANNEL A")]), Some(1));
    }

    #[test]
    fn describes_channel_counts() {
        use MemoryType::*;
        assert_eq!(describe(Ddr5, 1).as_deref(), Some("2 x 32-bit"));
        assert_eq!(describe(Ddr5, 2).as_deref(), Some("4 x 32-bit"));
        assert_eq!(describe(Ddr5, 8).as_deref(), Some("16 x 32-bit"));
        assert_eq!(describe(Ddr4, 1).as_deref(), Some("Single"));
        assert_eq!(describe(Ddr4, 2).as_deref(), Some("Dual"));
        assert_eq!(describe(Ddr3, 3).as_deref(), Some("Triple"));
        assert_eq!(describe(Ddr4, 4).as_deref(), Some("Quad"));
        assert_eq!(describe(Ddr4, 6).as_deref(), Some("Hexa"));
        assert_eq!(describe(Ddr4, 8).as_deref(), Some("Octa"));
        assert_eq!(describe(Lpddr3, 2).as_deref(), Some("Dual"));
        assert_eq!(describe(Ddr4, 7), None);
        assert_eq!(describe(Ddr4, 12), None);
        assert_eq!(describe(Ddr4, 0), None);
        assert_eq!(describe(Ddr5, 0), None);
        assert_eq!(describe(Lpddr4, 2), None);
        assert_eq!(describe(Lpddr5, 4), None);
        assert_eq!(describe(Other("SDRAM"), 1), None);
    }
}
