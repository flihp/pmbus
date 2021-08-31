#![no_std]

//! pmbus: A crate for PMBus manipulation
//!
//! This is a no_std crate that expresses the PMBus protocol, as described in
//! the PMBus 1.3 specifcation.  This crate is intended to be generic with
//! respect to implementation and usable by software that will directly
//! communicate with PMBus devices via SMBus/I2C as well as by software that
//! merely wishes to make sense of the PMBus protocol (e.g., debuggers or
//! analyzers running on a host).  For PMBus, this can be a bit of a
//! challenge, as much of the definition is left up to a particular device
//! (that is, much is implementation-defined).  Our two use cases are
//! therefore divergent in their needs:
//!
//! 1. The embedded system that is speaking to a particular PMBus device *in
//!    situ* is likely to know (and want to use) the special capabilities of a
//!    given device.  That is, these use cases know their target device at
//!    compile time, and have no need or desire to dynamically discover their
//!    device capabilities.
//!
//! 2. The host-based system that is trying to make sense of PMBus is *not*
//!    necessarily going to know the specifics of the attached devices at
//!    compile time; it is going to want to allow the device to be specified
//!    (or otherwise dynamically determined) and then discover that device's
//!    capabilities dynamically -- even if only to pass those capabilities on
//!    to the user.
//!
//! These use cases are in tension:  we want the first to be tight and
//! typesafe while still allowing for the more dynamic second use case.  We
//! balance these two cases by dynamically compiling the crate based on
//! per-device RON files that specify the commands and their corresponding
//! destructured data; each device is in its own module, with each PMBus
//! command further having its own module that contains the types for the
//! corresponding command data.
//!
//! As a concrete example, [`commands::OPERATION`] contains an implementation
//! of the [`commands::CommandData`] trait for the fields for the common PMBus
//! `OPERATION` command.  For each device, there is a device-specific
//! `OPERATION` module -- e.g.  `[commands::adm1272::OPERATION]` -- that may
//! extend or override the common definition.  Further, the device may define
//! its own constants; for example, while PMBus defines the command code
//! `0xd4` to be [`CommandCode::MFR_SPECIFIC_D4`], the ADM1272 defines this to
//! be `PMON_CONFIG`, a device-specific power monitor configuration register.
//! There therefore exists a [`commands::adm1272::PMON_CONFIG`] module that
//! understands the full (ADM1272-specific) functionality.  For code that
//! wishes to be device agnostic but still be able to display contents, there
//! exists a [`Device::interpret`] that given a device, a code, and a payload,
//! calls the specified closure to iterate over fields and values.  
//!
//! A final (crucial) constraint is that this crate remains `no_std`; it
//! performs no dynamic allocation and in general relies on program text
//! rather than table lookups -- with the knowledge that the compiler is very
//! good about dead code elimination and will not include unused program text
//! in the embedded system.
//!
//! If it needs to be said:  all of this adds up to specifications almost
//! entirely via RON definitions -- and an absolutely unholy `build.rs` to
//! assemble it all at build time.  Paraphrasing [the late Roger
//! Faulker](https://www.usenix.org/memoriam-roger-faulkner),
//! terrible things are sometimes required for beautiful abstractions.
//!

pub use num_derive::{FromPrimitive, ToPrimitive};
pub use num_traits::float::FloatCore;
pub use num_traits::{FromPrimitive, ToPrimitive};

mod operation;
pub use crate::operation::Operation;

pub mod units;

pub mod commands;
pub use crate::commands::devices;
pub use crate::commands::{
    Bitpos, Bitwidth, Command, CommandCode, CommandData, Device, Error, Field,
    Value,
};

///
/// The coefficients spelled out by PMBus for use in the DIRECT data format
/// (Part II, Sec. 7.4). The actual values used will depend on the device and
/// the condition.
///
#[derive(Copy, Clone, PartialEq, Debug)]
#[allow(non_snake_case)]
pub struct Coefficients {
    /// Slope coefficient. Two byte signed off the wire (but potentially
    /// larger after adjustment).
    pub m: i32,
    /// Offset. Two-byte, signed.
    pub b: i16,
    /// Exponent. One-byte, signed.
    pub R: i8,
}

///
/// A datum in the DIRECT data format.
///
#[derive(Copy, Clone, Debug)]
pub struct Direct(pub u16, pub Coefficients);

impl Direct {
    #[allow(dead_code)]
    pub fn to_real(&self) -> f32 {
        let coefficients = &self.1;
        let m: f32 = coefficients.m as f32;
        let b: f32 = coefficients.b.into();
        let exp: i32 = coefficients.R.into();
        let y: f32 = (self.0 as i16).into();

        (y * f32::powi(10.0, -exp) - b) / m
    }

    #[allow(dead_code)]
    pub fn from_real(x: f32, coefficients: Coefficients) -> Self {
        let m: f32 = coefficients.m as f32;
        let b: f32 = coefficients.b.into();
        let exp: i32 = coefficients.R.into();
        let y: f32 = (m * x + b) * f32::powi(10.0, exp);

        Self(y.round() as u16, coefficients)
    }
}

///
/// A datum in the LINEAR11 data format.
///
#[derive(Copy, Clone, Debug)]
pub struct Linear11(pub u16);

//
// The LINEAR11 format is outlined in Section 7.3 of the PMBus specification.
// It consists of 5 bits of signed exponent (N), and 11 bits of signed mantissa
// (Y):
//
// |<------------ high byte ------------>|<--------- low byte ---------->|
// +---+---+---+---+---+     +---+---+---+---+---+---+---+---+---+---+---+
// | 7 | 6 | 5 | 4 | 3 |     | 2 | 1 | 0 | 7 | 6 | 5 | 4 | 3 | 2 | 1 | 0 |
// +---+---+---+---+---+     +---+---+---+---+---+---+---+---+---+---+---+
//
// |<------- N ------->|     |<------------------- Y ------------------->|
//
// The relation between these values and the real world value is:
//
//   X = Y * 2^N
//
const LINEAR11_Y_WIDTH: u16 = 11;
const LINEAR11_Y_MAX: i16 = (1 << (LINEAR11_Y_WIDTH - 1)) - 1;
const LINEAR11_Y_MIN: i16 = -(1 << (LINEAR11_Y_WIDTH - 1));
const LINEAR11_Y_MASK: i16 = (1 << LINEAR11_Y_WIDTH) - 1;

const LINEAR11_N_WIDTH: u16 = 5;
const LINEAR11_N_MAX: i16 = (1 << (LINEAR11_N_WIDTH - 1)) - 1;
const LINEAR11_N_MIN: i16 = -(1 << (LINEAR11_N_WIDTH - 1));
const LINEAR11_N_MASK: i16 = (1 << LINEAR11_N_WIDTH) - 1;

impl Linear11 {
    pub fn to_real(&self) -> f32 {
        let n = (self.0 as i16) >> LINEAR11_Y_WIDTH;
        let y = ((self.0 << LINEAR11_N_WIDTH) as i16) >> LINEAR11_N_WIDTH;

        y as f32 * f32::powi(2.0, n.into())
    }

    #[allow(dead_code)]
    pub fn from_real(x: f32) -> Option<Self> {
        //
        // We get our closest approximation when we have as many digits as
        // possible in Y; to determine the value of N that will satisfy this,
        // we pick a value of Y that is further away from 0 (more positive or
        // more negative) than our true Y and determine what N would be, taking
        // the ceiling of this value.  If this value exceeds our resolution for
        // N, we cannot represent the value.
        //
        let n = if x >= 0.0 {
            x / LINEAR11_Y_MAX as f32
        } else {
            x / LINEAR11_Y_MIN as f32
        };

        let n = f32::ceil(libm::log2f(n)) as i16;

        if n < LINEAR11_N_MIN || n > LINEAR11_N_MAX {
            None
        } else {
            let exp = f32::powi(2.0, n.into());
            let y = x / exp;

            let high = ((n & LINEAR11_N_MASK) as u16) << LINEAR11_Y_WIDTH;
            let low = ((y as i16) & LINEAR11_Y_MASK) as u16;

            Some(Linear11(high | low))
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ULinear16Exponent(pub i8);

///
/// A datum in the ULINEAR16 format.  ULINEAR16 is used only for voltage;
/// the exponent comes from VOUT_MODE.
///
pub struct ULinear16(pub u16, pub ULinear16Exponent);

impl ULinear16 {
    pub fn to_real(&self) -> f32 {
        let exp = self.1 .0;
        self.0 as f32 * f32::powi(2.0, exp.into())
    }

    pub fn from_real(x: f32, exp: ULinear16Exponent) -> Option<Self> {
        let val = (x / f32::powi(2.0, exp.0.into())).round();

        if val > core::u16::MAX as f32 {
            None
        } else {
            Some(Self(val as u16, exp))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    fn mode() -> commands::VOutMode {
        panic!("unexpected call to get VOutMode");
    }

    #[test]
    fn verify_cmds() {
        macro_rules! verify {
            ($val:expr, $cmd:tt, $write:tt, $read:tt) => {
                assert_eq!(CommandCode::$cmd as u8, $val);
                assert_eq!(CommandCode::$cmd.write_op(), Operation::$write);
                assert_eq!(CommandCode::$cmd.read_op(), Operation::$read);
            };
        }

        //
        // This is deliberately designed to allow one to read Table 31 in the
        // 1.3.1 specification and validate it.
        //
        verify!(0x00, PAGE, WriteByte, ReadByte);
        verify!(0x01, OPERATION, WriteByte, ReadByte);
        verify!(0x02, ON_OFF_CONFIG, WriteByte, ReadByte);
        verify!(0x03, CLEAR_FAULTS, SendByte, Illegal);
        verify!(0x04, PHASE, WriteByte, ReadByte);
        verify!(0x05, PAGE_PLUS_WRITE, WriteBlock, Illegal);
        verify!(0x06, PAGE_PLUS_READ, Illegal, ProcessCall);
        verify!(0x07, ZONE_CONFIG, WriteWord, ReadWord);
        verify!(0x08, ZONE_ACTIVE, WriteWord, ReadWord);
        verify!(0x10, WRITE_PROTECT, WriteByte, ReadByte);
        verify!(0x11, STORE_DEFAULT_ALL, SendByte, Illegal);
        verify!(0x12, RESTORE_DEFAULT_ALL, SendByte, Illegal);
        verify!(0x13, STORE_DEFAULT_CODE, WriteByte, Illegal);
        verify!(0x14, RESTORE_DEFAULT_CODE, WriteByte, Illegal);
        verify!(0x15, STORE_USER_ALL, SendByte, Illegal);
        verify!(0x16, RESTORE_USER_ALL, SendByte, Illegal);
        verify!(0x17, STORE_USER_CODE, WriteByte, Illegal);
        verify!(0x18, RESTORE_USER_CODE, WriteByte, Illegal);
        verify!(0x19, CAPABILITY, Illegal, ReadByte);
        verify!(0x1a, QUERY, Illegal, ProcessCall);
        verify!(0x1b, SMBALERT_MASK, WriteWord, ProcessCall);
        verify!(0x20, VOUT_MODE, WriteByte, ReadByte);
        verify!(0x21, VOUT_COMMAND, WriteWord, ReadWord);
        verify!(0x22, VOUT_TRIM, WriteWord, ReadWord);
        verify!(0x23, VOUT_CAL_OFFSET, WriteWord, ReadWord);
        verify!(0x24, VOUT_MAX, WriteWord, ReadWord);
        verify!(0x25, VOUT_MARGIN_HIGH, WriteWord, ReadWord);
        verify!(0x26, VOUT_MARGIN_LOW, WriteWord, ReadWord);
        verify!(0x27, VOUT_TRANSITION_RATE, WriteWord, ReadWord);
        verify!(0x28, VOUT_DROOP, WriteWord, ReadWord);
        verify!(0x29, VOUT_SCALE_LOOP, WriteWord, ReadWord);
        verify!(0x2a, VOUT_SCALE_MONITOR, WriteWord, ReadWord);
        verify!(0x2b, VOUT_MIN, WriteWord, ReadWord);
        verify!(0x30, COEFFICIENTS, Illegal, ProcessCall);
        verify!(0x31, POUT_MAX, WriteWord, ReadWord);
        verify!(0x32, MAX_DUTY, WriteWord, ReadWord);
        verify!(0x33, FREQUENCY_SWITCH, WriteWord, ReadWord);
        verify!(0x34, POWER_MODE, WriteByte, ReadByte);
        verify!(0x35, VIN_ON, WriteWord, ReadWord);
        verify!(0x36, VIN_OFF, WriteWord, ReadWord);
        verify!(0x37, INTERLEAVE, WriteWord, ReadWord);
        verify!(0x38, IOUT_CAL_GAIN, WriteWord, ReadWord);
        verify!(0x39, IOUT_CAL_OFFSET, WriteWord, ReadWord);
        verify!(0x3a, FAN_CONFIG_1_2, WriteByte, ReadByte);
        verify!(0x3b, FAN_COMMAND_1, WriteWord, ReadWord);
        verify!(0x3c, FAN_COMMAND_2, WriteWord, ReadWord);
        verify!(0x3d, FAN_CONFIG_3_4, WriteByte, ReadByte);
        verify!(0x3e, FAN_COMMAND_3, WriteWord, ReadWord);
        verify!(0x3f, FAN_COMMAND_4, WriteWord, ReadWord);
        verify!(0x40, VOUT_OV_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x41, VOUT_OV_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x42, VOUT_OV_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x43, VOUT_UV_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x44, VOUT_UV_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x45, VOUT_UV_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x46, IOUT_OC_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x47, IOUT_OC_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x48, IOUT_OC_LV_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x49, IOUT_OC_LV_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x4a, IOUT_OC_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x4b, IOUT_UC_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x4c, IOUT_UC_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x4f, OT_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x50, OT_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x51, OT_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x52, UT_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x53, UT_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x54, UT_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x55, VIN_OV_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x56, VIN_OV_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x57, VIN_OV_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x58, VIN_UV_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x59, VIN_UV_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x5a, VIN_UV_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x5b, IIN_OC_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x5c, IIN_OC_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x5d, IIN_OC_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x5e, POWER_GOOD_ON, WriteWord, ReadWord);
        verify!(0x5f, POWER_GOOD_OFF, WriteWord, ReadWord);
        verify!(0x60, TON_DELAY, WriteWord, ReadWord);
        verify!(0x61, TON_RISE, WriteWord, ReadWord);
        verify!(0x62, TON_MAX_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x63, TON_MAX_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x64, TOFF_DELAY, WriteWord, ReadWord);
        verify!(0x65, TOFF_FALL, WriteWord, ReadWord);
        verify!(0x66, TOFF_MAX_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x67, Deprecated, Unknown, Unknown);
        verify!(0x68, POUT_OP_FAULT_LIMIT, WriteWord, ReadWord);
        verify!(0x69, POUT_OP_FAULT_RESPONSE, WriteByte, ReadByte);
        verify!(0x6a, POUT_OP_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x6b, PIN_OP_WARN_LIMIT, WriteWord, ReadWord);
        verify!(0x78, STATUS_BYTE, WriteByte, ReadByte);
        verify!(0x79, STATUS_WORD, WriteWord, ReadWord);
        verify!(0x7a, STATUS_VOUT, WriteByte, ReadByte);
        verify!(0x7b, STATUS_IOUT, WriteByte, ReadByte);
        verify!(0x7c, STATUS_INPUT, WriteByte, ReadByte);
        verify!(0x7d, STATUS_TEMPERATURE, WriteByte, ReadByte);
        verify!(0x7e, STATUS_CML, WriteByte, ReadByte);
        verify!(0x7f, STATUS_OTHER, WriteByte, ReadByte);
        verify!(0x80, STATUS_MFR_SPECIFIC, WriteByte, ReadByte);
        verify!(0x81, STATUS_FANS_1_2, WriteByte, ReadByte);
        verify!(0x82, STATUS_FANS_3_4, WriteByte, ReadByte);
        verify!(0x83, READ_KWH_IN, Illegal, ReadWord32);
        verify!(0x84, READ_KWH_OUT, Illegal, ReadWord32);
        verify!(0x85, READ_KWH_CONFIG, WriteWord, ReadWord);
        verify!(0x86, READ_EIN, Illegal, ReadBlock);
        verify!(0x87, READ_EOUT, Illegal, ReadBlock);
        verify!(0x88, READ_VIN, Illegal, ReadWord);
        verify!(0x89, READ_IIN, Illegal, ReadWord);
        verify!(0x8a, READ_VCAP, Illegal, ReadWord);
        verify!(0x8b, READ_VOUT, Illegal, ReadWord);
        verify!(0x8c, READ_IOUT, Illegal, ReadWord);
        verify!(0x8d, READ_TEMPERATURE_1, Illegal, ReadWord);
        verify!(0x8e, READ_TEMPERATURE_2, Illegal, ReadWord);
        verify!(0x8f, READ_TEMPERATURE_3, Illegal, ReadWord);
        verify!(0x90, READ_FAN_SPEED_1, Illegal, ReadWord);
        verify!(0x91, READ_FAN_SPEED_2, Illegal, ReadWord);
        verify!(0x92, READ_FAN_SPEED_3, Illegal, ReadWord);
        verify!(0x93, READ_FAN_SPEED_4, Illegal, ReadWord);
        verify!(0x94, READ_DUTY_CYCLE, Illegal, ReadWord);
        verify!(0x95, READ_FREQUENCY, Illegal, ReadWord);
        verify!(0x96, READ_POUT, Illegal, ReadWord);
        verify!(0x97, READ_PIN, Illegal, ReadWord);
        verify!(0x98, PMBUS_REVISION, Illegal, ReadByte);
        verify!(0x99, MFR_ID, WriteBlock, ReadBlock);
        verify!(0x9a, MFR_MODEL, WriteBlock, ReadBlock);
        verify!(0x9b, MFR_REVISION, WriteBlock, ReadBlock);
        verify!(0x9c, MFR_LOCATION, WriteBlock, ReadBlock);
        verify!(0x9d, MFR_DATE, WriteBlock, ReadBlock);
        verify!(0x9e, MFR_SERIAL, WriteBlock, ReadBlock);
        verify!(0x9f, APP_PROFILE_SUPPORT, Illegal, ReadBlock);
        verify!(0xa0, MFR_VIN_MIN, Illegal, ReadWord);
        verify!(0xa1, MFR_VIN_MAX, Illegal, ReadWord);
        verify!(0xa2, MFR_IIN_MAX, Illegal, ReadWord);
        verify!(0xa3, MFR_PIN_MAX, Illegal, ReadWord);
        verify!(0xa4, MFR_VOUT_MIN, Illegal, ReadWord);
        verify!(0xa5, MFR_VOUT_MAX, Illegal, ReadWord);
        verify!(0xa6, MFR_IOUT_MAX, Illegal, ReadWord);
        verify!(0xa7, MFR_POUT_MAX, Illegal, ReadWord);
        verify!(0xa8, MFR_TAMBIENT_MAX, Illegal, ReadWord);
        verify!(0xa9, MFR_TAMBIENT_MIN, Illegal, ReadWord);
        verify!(0xaa, MFR_EFFICIENCY_LL, Illegal, ReadBlock);
        verify!(0xab, MFR_EFFICIENCY_HL, Illegal, ReadBlock);
        verify!(0xac, MFR_PIN_ACCURACY, Illegal, ReadByte);
        verify!(0xad, IC_DEVICE_ID, Illegal, ReadBlock);
        verify!(0xae, IC_DEVICE_REV, Illegal, ReadBlock);
        verify!(0xb0, USER_DATA_00, WriteBlock, ReadBlock);
        verify!(0xb1, USER_DATA_01, WriteBlock, ReadBlock);
        verify!(0xb2, USER_DATA_02, WriteBlock, ReadBlock);
        verify!(0xb3, USER_DATA_03, WriteBlock, ReadBlock);
        verify!(0xb4, USER_DATA_04, WriteBlock, ReadBlock);
        verify!(0xb5, USER_DATA_05, WriteBlock, ReadBlock);
        verify!(0xb6, USER_DATA_06, WriteBlock, ReadBlock);
        verify!(0xb7, USER_DATA_07, WriteBlock, ReadBlock);
        verify!(0xb8, USER_DATA_08, WriteBlock, ReadBlock);
        verify!(0xb9, USER_DATA_09, WriteBlock, ReadBlock);
        verify!(0xba, USER_DATA_10, WriteBlock, ReadBlock);
        verify!(0xbb, USER_DATA_11, WriteBlock, ReadBlock);
        verify!(0xbc, USER_DATA_12, WriteBlock, ReadBlock);
        verify!(0xbd, USER_DATA_13, WriteBlock, ReadBlock);
        verify!(0xbe, USER_DATA_14, WriteBlock, ReadBlock);
        verify!(0xbf, USER_DATA_15, WriteBlock, ReadBlock);
        verify!(0xc0, MFR_MAX_TEMP_1, WriteWord, ReadWord);
        verify!(0xc1, MFR_MAX_TEMP_2, WriteWord, ReadWord);
        verify!(0xc2, MFR_MAX_TEMP_3, WriteWord, ReadWord);
        verify!(0xc4, MFR_SPECIFIC_C4, MfrDefined, MfrDefined);
        verify!(0xc4, MFR_SPECIFIC_C4, MfrDefined, MfrDefined);
        verify!(0xc5, MFR_SPECIFIC_C5, MfrDefined, MfrDefined);
        verify!(0xc6, MFR_SPECIFIC_C6, MfrDefined, MfrDefined);
        verify!(0xc7, MFR_SPECIFIC_C7, MfrDefined, MfrDefined);
        verify!(0xc8, MFR_SPECIFIC_C8, MfrDefined, MfrDefined);
        verify!(0xc9, MFR_SPECIFIC_C9, MfrDefined, MfrDefined);
        verify!(0xca, MFR_SPECIFIC_CA, MfrDefined, MfrDefined);
        verify!(0xcb, MFR_SPECIFIC_CB, MfrDefined, MfrDefined);
        verify!(0xcc, MFR_SPECIFIC_CC, MfrDefined, MfrDefined);
        verify!(0xcd, MFR_SPECIFIC_CD, MfrDefined, MfrDefined);
        verify!(0xce, MFR_SPECIFIC_CE, MfrDefined, MfrDefined);
        verify!(0xcf, MFR_SPECIFIC_CF, MfrDefined, MfrDefined);
        verify!(0xd0, MFR_SPECIFIC_D0, MfrDefined, MfrDefined);
        verify!(0xd1, MFR_SPECIFIC_D1, MfrDefined, MfrDefined);
        verify!(0xd2, MFR_SPECIFIC_D2, MfrDefined, MfrDefined);
        verify!(0xd3, MFR_SPECIFIC_D3, MfrDefined, MfrDefined);
        verify!(0xd4, MFR_SPECIFIC_D4, MfrDefined, MfrDefined);
        verify!(0xd5, MFR_SPECIFIC_D5, MfrDefined, MfrDefined);
        verify!(0xd6, MFR_SPECIFIC_D6, MfrDefined, MfrDefined);
        verify!(0xd7, MFR_SPECIFIC_D7, MfrDefined, MfrDefined);
        verify!(0xd8, MFR_SPECIFIC_D8, MfrDefined, MfrDefined);
        verify!(0xd9, MFR_SPECIFIC_D9, MfrDefined, MfrDefined);
        verify!(0xda, MFR_SPECIFIC_DA, MfrDefined, MfrDefined);
        verify!(0xdb, MFR_SPECIFIC_DB, MfrDefined, MfrDefined);
        verify!(0xdc, MFR_SPECIFIC_DC, MfrDefined, MfrDefined);
        verify!(0xdd, MFR_SPECIFIC_DD, MfrDefined, MfrDefined);
        verify!(0xde, MFR_SPECIFIC_DE, MfrDefined, MfrDefined);
        verify!(0xdf, MFR_SPECIFIC_DF, MfrDefined, MfrDefined);
        verify!(0xe0, MFR_SPECIFIC_E0, MfrDefined, MfrDefined);
        verify!(0xe1, MFR_SPECIFIC_E1, MfrDefined, MfrDefined);
        verify!(0xe2, MFR_SPECIFIC_E2, MfrDefined, MfrDefined);
        verify!(0xe3, MFR_SPECIFIC_E3, MfrDefined, MfrDefined);
        verify!(0xe4, MFR_SPECIFIC_E4, MfrDefined, MfrDefined);
        verify!(0xe5, MFR_SPECIFIC_E5, MfrDefined, MfrDefined);
        verify!(0xe6, MFR_SPECIFIC_E6, MfrDefined, MfrDefined);
        verify!(0xe7, MFR_SPECIFIC_E7, MfrDefined, MfrDefined);
        verify!(0xe8, MFR_SPECIFIC_E8, MfrDefined, MfrDefined);
        verify!(0xe9, MFR_SPECIFIC_E9, MfrDefined, MfrDefined);
        verify!(0xea, MFR_SPECIFIC_EA, MfrDefined, MfrDefined);
        verify!(0xeb, MFR_SPECIFIC_EB, MfrDefined, MfrDefined);
        verify!(0xec, MFR_SPECIFIC_EC, MfrDefined, MfrDefined);
        verify!(0xed, MFR_SPECIFIC_ED, MfrDefined, MfrDefined);
        verify!(0xee, MFR_SPECIFIC_EE, MfrDefined, MfrDefined);
        verify!(0xef, MFR_SPECIFIC_EF, MfrDefined, MfrDefined);
        verify!(0xf0, MFR_SPECIFIC_F0, MfrDefined, MfrDefined);
        verify!(0xf1, MFR_SPECIFIC_F1, MfrDefined, MfrDefined);
        verify!(0xf2, MFR_SPECIFIC_F2, MfrDefined, MfrDefined);
        verify!(0xf3, MFR_SPECIFIC_F3, MfrDefined, MfrDefined);
        verify!(0xf4, MFR_SPECIFIC_F4, MfrDefined, MfrDefined);
        verify!(0xf5, MFR_SPECIFIC_F5, MfrDefined, MfrDefined);
        verify!(0xf6, MFR_SPECIFIC_F6, MfrDefined, MfrDefined);
        verify!(0xf7, MFR_SPECIFIC_F7, MfrDefined, MfrDefined);
        verify!(0xf8, MFR_SPECIFIC_F8, MfrDefined, MfrDefined);
        verify!(0xf9, MFR_SPECIFIC_F9, MfrDefined, MfrDefined);
        verify!(0xfa, MFR_SPECIFIC_FA, MfrDefined, MfrDefined);
        verify!(0xfb, MFR_SPECIFIC_FB, MfrDefined, MfrDefined);
        verify!(0xfc, MFR_SPECIFIC_FC, MfrDefined, MfrDefined);
        verify!(0xfd, MFR_SPECIFIC_FD, MfrDefined, MfrDefined);
        verify!(0xfe, MFR_SPECIFIC_COMMAND_EXT, Extended, Extended);
        verify!(0xff, PMBUS_COMMAND_EXT, Extended, Extended);
        std::println!("{:?}", CommandCode::from_u8(0x9));
    }

    #[test]
    fn verify_operation() {
        let data = commands::OPERATION::CommandData(0x4);

        data.interpret(mode, |field, value| {
            std::println!("{} = {}", field.desc(), value);
        })
        .unwrap();
    }

    #[test]
    fn verify_operation_set() {
        use commands::OPERATION::*;
        let mut data = CommandData(0x4);

        dump(&data);

        assert_ne!(
            data.get_voltage_command_source(),
            Some(VoltageCommandSource::VOUT_MARGIN_HIGH)
        );

        data.set_voltage_command_source(VoltageCommandSource::VOUT_MARGIN_HIGH);

        dump(&data);

        assert_eq!(
            data.get_voltage_command_source(),
            Some(VoltageCommandSource::VOUT_MARGIN_HIGH)
        );
    }

    #[test]
    fn raw_operation() {
        CommandCode::OPERATION
            .interpret(&[0x4], mode, |field, value| {
                std::println!("{} = {}", field.desc(), value);
            })
            .unwrap();
    }

    fn dump_data(
        val: u32,
        width: Bitwidth,
        v: &mut std::vec::Vec<((Bitpos, Bitwidth), &str, std::string::String)>,
    ) {
        let width = width.0 as usize;
        let nibble = 4;
        let maxwidth = 16;

        if width > maxwidth {
            std::println!("{:?}", v);
            return;
        }

        let indent = (maxwidth - width) + ((maxwidth - width) / nibble);

        std::print!("{:indent$}", "", indent = indent);
        std::print!("0b");

        for v in (0..width).step_by(nibble) {
            std::print!(
                "{:04b}{}",
                (val >> ((width - nibble) - v)) & 0xf,
                if v + nibble < width { "_" } else { "\n" }
            )
        }

        while v.len() > 0 {
            let mut cur = width - 1;

            std::print!("{:indent$}", "", indent = indent);
            std::print!("  ");

            for i in 0..v.len() {
                while cur > v[i].0 .0 .0 as usize {
                    if cur % nibble == 0 {
                        std::print!(" ");
                    }

                    std::print!(" ");
                    cur -= 1;
                }

                if i < v.len() - 1 {
                    std::print!("|");

                    if cur % nibble == 0 {
                        std::print!(" ");
                    }

                    cur -= 1;
                } else {
                    std::print!("+--");

                    while cur > 0 {
                        std::print!("-");

                        if cur % nibble == 0 {
                            std::print!("-");
                        }

                        cur -= 1;
                    }

                    std::println!(" {} = {}", v[i].1, v[i].2);
                }
            }

            v.pop();
        }
    }

    fn dump(data: &impl commands::CommandData) {
        let (val, width) = data.raw();
        let mut v = std::vec![];

        data.command(|cmd| {
            std::println!("\n{:?}: ", cmd);
        });

        data.interpret(mode, |field, value| {
            v.push((field.bits(), field.desc(), std::format!("{}", value)));
        })
        .unwrap();

        dump_data(val, width, &mut v);
    }

    #[test]
    fn verify_status_word() {
        use commands::STATUS_WORD::*;

        let data = CommandData::from_slice(&[0x43, 0x18]).unwrap();
        dump(&data);

        data.interpret(mode, |field, value| {
            std::println!("{} = {}", field.desc(), value);
        })
        .unwrap();
    }

    #[test]
    fn verify_on_off_config() {
        use commands::ON_OFF_CONFIG::*;

        let data = CommandData::from_slice(&[0x17]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_capability() {
        use commands::CAPABILITY::*;

        let data = CommandData::from_slice(&[0xd0]).unwrap();
        dump(&data);

        let data = CommandData::from_slice(&[0xb0]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_vout_mode() {
        use commands::VOUT_MODE::*;
        let data = CommandData::from_slice(&[0x97]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_status_vout() {
        use commands::STATUS_VOUT::*;
        let data = CommandData::from_slice(&[0x0]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_status_iout() {
        use commands::STATUS_IOUT::*;
        let data = CommandData::from_slice(&[0x0]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_status_cml() {
        use commands::STATUS_CML::*;
        let data = CommandData::from_slice(&[0x82]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_status_other() {
        use commands::STATUS_OTHER::*;
        let data = CommandData::from_slice(&[0x1]).unwrap();
        dump(&data);
    }

    #[test]
    fn verify_status_adm1272() {
        use commands::adm1272::STATUS_MFR_SPECIFIC::*;
        let data = CommandData::from_slice(&[0x40]).unwrap();
        dump(&data);
    }

    #[test]
    fn device_list() {
        let code = commands::CommandCode::STATUS_MFR_SPECIFIC as u8;

        std::println!("code is {:x}", code);

        devices(|d| {
            for i in 0..=0xff {
                d.command(i, |cmd| {
                    std::println!(
                        "{:?}: {:2x} {} R={:?} W={:?}",
                        d,
                        i,
                        cmd.name(),
                        cmd.read_op(),
                        cmd.write_op()
                    );
                });
            }
        });
    }

    #[test]
    fn tps_read_all() {
        use commands::tps546b24a::READ_ALL::*;

        let data = CommandData::from_slice(&[
            0x02, 0x00, 0x63, 0x02, 0xee, 0xad, 0xd8, 0xdb, 0xfe, 0xd2, 0x00,
            0x00, 0x00, 0x00,
        ])
        .unwrap();

        assert_eq!(data.get_read_vin(), 0xd2fe);
        assert_eq!(data.get_read_vout(), 0x0263);
        assert_eq!(data.get_status_word(), 0x0002);
        assert_eq!(data.get_read_temperature_1(), 0xdbd8);
    }

    #[test]
    fn tps_read_all_data() {
        let _code = commands::tps546b24a::CommandCode::READ_ALL as u8;
        let mode = || commands::VOutMode::from_slice(&[0x97]).unwrap();

        let data = [
            0x02, 0x00, 0x63, 0x02, 0xee, 0xad, 0xd8, 0xdb, 0xfe, 0xd2, 0x00,
            0x00, 0x00, 0x00,
        ];

        for code in 0..=0xff {
            let _ = Device::Tps546B24A.interpret(
                code,
                &data[0..],
                mode,
                |f, _v| {
                    std::println!("f is {}", f.desc());
                },
            );
        }
    }

    #[test]
    fn tps_passthrough() {
        //
        // This is a bit of a mouthful of a test to assure that common registers
        // are correctly passed through into device-specific modules.
        //
        use commands::tps546b24a::CAPABILITY::*;

        let code = commands::tps546b24a::CommandCode::CAPABILITY as u8;
        let payload = &[0xd0];
        let mut result = None;

        let name = Field::MaximumBusSpeed.name();

        let cap = CommandData::from_slice(payload).unwrap();
        let val = cap.get(Field::MaximumBusSpeed).unwrap();
        let target = std::format!("{}", val);

        Device::Tps546B24A
            .interpret(code, payload, mode, |f, v| {
                if f.name() == name {
                    result = Some(std::format!("{}", v));
                }
            })
            .unwrap();

        assert_eq!(result, Some(target));
    }

    #[test]
    fn bmr480_default() {
        use commands::bmr480::*;

        let data = MFR_FAST_OCP_CFG::CommandData::from_slice(&[0xe9, 0x02]);
        dump(&data.unwrap());

        let data = MFR_RESPONSE_UNIT_CFG::CommandData::from_slice(&[0x51]);
        dump(&data.unwrap());

        let data = MFR_ISHARE_THRESHOLD::CommandData::from_slice(&[
            0x10, 0x10, 0x00, 0x64, 0x00, 0x00, 0x00, 0x01,
        ])
        .unwrap();

        assert_eq!(data.get_trim_limit(), units::Volts(0.170));

        dump(&data);
    }

    #[test]
    fn bmr491_default() {
        use commands::bmr491::*;

        let data = MFR_FAST_OCP_CFG::CommandData::from_slice(&[0xe9, 0x02]);
        dump(&data.unwrap());

        let data = MFR_RESPONSE_UNIT_CFG::CommandData::from_slice(&[0x51]);
        dump(&data.unwrap());

        let mut data = MFR_ISHARE_THRESHOLD::CommandData::from_slice(&[
            0x10, 0x10, 0x00, 0x64, 0x00, 0x00, 0x00, 0x01,
        ])
        .unwrap();

        assert_eq!(data.get_trim_limit(), units::Volts(0.170));

        assert_eq!(data.set_trim_limit(units::Volts(0.136)), Ok(()));
        assert_eq!(data.get_trim_limit(), units::Volts(0.136));

        dump(&data);
    }

    #[test]
    fn bmr480_iout() {
        use commands::bmr480::*;

        let data = [
            (0xf028u16, 10.0),
            (0xf133, 76.75),
            (0xf040, 16.0),
            (0xf004, 1.0),
            (0xf051, 20.25),
            (0xf079, 30.25),
            (0xf00a, 2.5),
            (0xf0c9, 50.25),
            (0xf07d, 31.25),
            (0xf00b, 2.75),
            (0xf009, 2.25),
        ];

        for d in data {
            let raw = d.0.to_le_bytes();
            let iout = READ_IOUT::CommandData::from_slice(&raw).unwrap();
            assert_eq!(iout.get(), Ok(units::Amperes(d.1)));

            iout.interpret(mode, |f, v| {
                assert_eq!(f.bitfield(), false);
                std::println!("{} 0x{:04x} = {}", f.name(), d.0, v);
            })
            .unwrap();
        }
    }

    #[test]
    fn bmr480_vout() {
        use commands::bmr480::*;

        let mode = || commands::VOutMode::from_slice(&[0x15]).unwrap();

        let data = [
            (0x0071u16, 0.05517578f32),
            (0x0754, 0.9160156),
            (0x5f72, 11.930664),
            (0x5f80, 11.9375),
            (0x5fd3, 11.978027),
            (0x5fdb, 11.981934),
            (0x5fe4, 11.986328),
            (0x5fe6, 11.987305),
            (0x5fec, 11.990234),
            (0x5fee, 11.991211),
            (0x5ff7, 11.995605),
            (0x6007, 12.003418),
            (0x6039, 12.027832),
            (0x603f, 12.030762),
            (0x6091, 12.070801),
            (0x65b7, 12.714355),
            (0x65d8, 12.730469),
            (0x670a, 12.879883),
            (0x68b0, 13.0859375),
            (0x69c1, 13.219238),
            (0x69e2, 13.235352),
        ];

        for d in data {
            let raw = d.0.to_le_bytes();
            let vout = READ_VOUT::CommandData::from_slice(&raw).unwrap();
            assert_eq!(vout.get(mode()), Ok(units::Volts(d.1)));

            vout.interpret(mode, |f, v| {
                assert_eq!(f.bitfield(), false);
                std::println!("{} 0x{:04x} = {}", f.name(), d.0, v);
            })
            .unwrap();
        }
    }

    #[test]
    fn bmr480_vin() {
        use commands::bmr480::*;

        let mode = || commands::VOutMode::from_slice(&[0x15]).unwrap();

        let data = [
            (0x0a5cu16, 1208.0),
            (0x0a8c, 1304.0),
            (0xe9a0, 52.0),
            (0xe9a1, 52.125),
            (0xe9a2, 52.25),
            (0xe9a3, 52.375),
            (0xe9a4, 52.5),
            (0xe9a6, 52.75),
            (0xe9a7, 52.875),
        ];

        for d in data {
            let raw = d.0.to_le_bytes();
            let vin = READ_VIN::CommandData::from_slice(&raw).unwrap();
            assert_eq!(vin.get(), Ok(units::Volts(d.1)));

            vin.interpret(mode, |f, v| {
                assert_eq!(f.bitfield(), false);
                std::println!("{} 0x{:04x} = {}", f.name(), d.0, v);
            })
            .unwrap();
        }
    }

    #[test]
    fn bmr491_rc_level() {
        use commands::bmr491::*;
        let rc = MFR_RC_LEVEL::CommandData::from_slice(&[0xc8]).unwrap();
        assert_eq!(rc.get(), Ok(units::Volts(20.0)));
    }

    #[test]
    fn bmr491_ks_pretrig() {
        use commands::bmr491::*;
        let ks = MFR_KS_PRETRIG::CommandData::from_slice(&[0x89]).unwrap();
        assert_eq!(ks.get(), Ok(units::Microseconds(61.649998)));
    }

    #[test]
    fn isl68224_vin() {
        use commands::isl68224::*;

        let mode = || commands::VOutMode::from_slice(&[0x40]).unwrap();

        let data = [(0x04a9u16, 11.929999), (0xffff, -0.01)];

        for d in data {
            let raw = d.0.to_le_bytes();
            let vin = READ_VIN::CommandData::from_slice(&raw).unwrap();
            assert_eq!(vin.get(), Ok(units::Volts(d.1)));

            vin.interpret(mode, |f, v| {
                assert_eq!(f.bitfield(), false);
                std::println!("{} 0x{:04x} = {}", f.name(), d.0, v);
            })
            .unwrap();
        }
    }

    #[test]
    fn isl68224_ton_rise() {
        use commands::isl68224::TON_RISE::*;

        let mut data = CommandData::from_slice(&[0xf4, 0x01]).unwrap();
        assert_eq!(data.get(), Ok(units::Milliseconds(0.5)));

        data.set(units::Milliseconds(0.75)).unwrap();
        assert_eq!(data.get(), Ok(units::Milliseconds(0.75000006)));

        data.mutate(mode, |field, _| {
            assert_eq!(field.bitfield(), false);
            assert_eq!(field.bits(), (Bitpos(0), Bitwidth(16)));
            Some(commands::Replacement::Float(0.25))
        })
        .unwrap();

        assert_eq!(data.get(), Ok(units::Milliseconds(0.25)));
    }

    #[test]
    fn mutate_operation() {
        use commands::OPERATION::*;

        let mut data = CommandData(0x4);
        dump(&data);

        std::println!("{:?}", data.get_on_off_state());
        assert_eq!(data.get_on_off_state(), Some(OnOffState::Off));

        data.mutate(mode, |field, _| {
            if field.name() == "OnOffState" {
                Some(commands::Replacement::Boolean(true))
            } else {
                None
            }
        })
        .unwrap();

        assert_eq!(data.get_on_off_state(), Some(OnOffState::On));

        dump(&data);
    }

    #[test]
    fn mutate_overflow_replacement() {
        use commands::OPERATION::*;

        let mut data = CommandData(0x4);

        let rval = data.mutate(mode, |field, _| {
            if field.name() == "OnOffState" {
                Some(commands::Replacement::Integer(3))
            } else {
                None
            }
        });

        assert_eq!(rval, Err(Error::OverflowReplacement));
    }

    #[test]
    fn mutate_invalid() {
        use commands::OPERATION::*;

        let mut data = CommandData(0x4);

        let rval = data.mutate(mode, |field, _| {
            if field.name() == "OnOffState" {
                Some(commands::Replacement::Float(3.1))
            } else {
                None
            }
        });

        assert_eq!(rval, Err(Error::InvalidReplacement));
    }

    #[test]
    fn vout_command_set() {
        let mut vout = commands::VOutMode::from_slice(&[0x97]).unwrap();
        use commands::VOUT_COMMAND::*;
        dump(&vout);

        std::println!("param is {}", vout.get_parameter());
        let mut data = CommandData::from_slice(&[0x63, 0x02]).unwrap();
        assert_eq!(data.get(vout), Ok(units::Volts(1.1933594)));

        data.set(vout, units::Volts(1.20)).unwrap();
        assert_eq!(data.0, 0x0266);
        assert_eq!(data.get(vout), Ok(units::Volts(1.1992188)));

        //
        // Now crank our resolution up
        //
        vout.set_parameter(-12).unwrap();
        assert_eq!(vout.get_parameter(), -12);
        data.set(vout, units::Volts(1.20)).unwrap();
        std::println!("{:?}", data.get(vout).unwrap());

        vout.set_parameter(-15).unwrap();
        assert_eq!(vout.get_parameter(), -15);
        data.set(vout, units::Volts(1.20)).unwrap();
        assert_eq!(data.get(vout), Ok(units::Volts(1.2000122)));

        //
        // With our exponent cranked to its maximum, there is no room
        // left for anything greater than 1.
        //
        vout.set_parameter(-16).unwrap();
        assert_eq!(vout.get_parameter(), -16);

        assert_eq!(vout.set_parameter(-101), Err(Error::ValueOutOfRange));
        std::println!("{:?}", vout.get_parameter());

        data.set(vout, units::Volts(0.20)).unwrap();
        assert_eq!(data.get(vout), Ok(units::Volts(0.19999695)));

        assert_eq!(
            data.set(vout, units::Volts(1.20)),
            Err(Error::ValueOutOfRange)
        );

        std::println!("{:?}", data.get(vout).unwrap());
    }

    #[test]
    fn vout_command_mutate() {
        let vout = commands::VOutMode::from_slice(&[0x97]).unwrap();
        use commands::VOUT_COMMAND::*;
        dump(&vout);

        let mut data = CommandData::from_slice(&[0x63, 0x02]).unwrap();
        assert_eq!(data.get(vout), Ok(units::Volts(1.1933594)));

        let rval = data.mutate(
            || vout,
            |field, _| {
                assert_eq!(field.bitfield(), false);
                assert_eq!(field.bits(), (Bitpos(0), Bitwidth(16)));
                Some(commands::Replacement::Float(1.20))
            },
        );

        assert_eq!(rval, Ok(()));
        assert_eq!(data.0, 0x0266);
        assert_eq!(data.get(vout), Ok(units::Volts(1.1992188)));

        let rval = data
            .mutate(|| vout, |_, _| Some(commands::Replacement::Integer(3)));

        assert_eq!(rval, Ok(()));
        assert_eq!(data.get(vout), Ok(units::Volts(3.0)));

        let rval = data
            .mutate(|| vout, |_, _| Some(commands::Replacement::Boolean(true)));

        assert_eq!(rval, Err(Error::InvalidReplacement));

        let rval = data
            .mutate(|| vout, |_, _| Some(commands::Replacement::Float(150.0)));

        assert_eq!(rval, Err(Error::ValueOutOfRange));
    }

    #[test]
    fn device_vout_command_mutate() {
        let vout = commands::VOutMode::from_slice(&[0x97]).unwrap();
        use commands::VOUT_COMMAND::*;
        dump(&vout);

        let mut payload = [0x63, 0x02];

        let data = CommandData::from_slice(&payload).unwrap();
        assert_eq!(data.get(vout), Ok(units::Volts(1.1933594)));

        let rval = Device::Common.mutate(
            commands::CommandCode::VOUT_COMMAND as u8,
            &mut payload[0..2],
            || vout,
            |field, _| {
                assert_eq!(field.bitfield(), false);
                assert_eq!(field.bits(), (Bitpos(0), Bitwidth(16)));
                Some(commands::Replacement::Float(1.20))
            },
        );

        assert_eq!(rval, Ok(()));
        assert_eq!(payload[0], 0x66);
        assert_eq!(payload[1], 0x02);

        let data = CommandData::from_slice(&payload).unwrap();
        assert_eq!(data.0, 0x0266);
        assert_eq!(data.get(vout), Ok(units::Volts(1.1992188)));
    }

    #[test]
    fn sentinels() {
        use commands::OPERATION::*;

        let data = CommandData::from_slice(&[0x88]).unwrap();
        dump(&data);

        CommandData::sentinels(Bitpos(4), |val| {
            match val.name() {
                "VOUT_COMMAND" => {
                    assert_eq!(val.raw(), 0);
                }
                "VOUT_MARGIN_LOW" => {
                    assert_eq!(val.raw(), 1);
                }
                "VOUT_MARGIN_HIGH" => {
                    assert_eq!(val.raw(), 2);
                }
                "AVS_VOUT_COMMAND" => {
                    assert_eq!(val.raw(), 3);
                }
                _ => {
                    panic!("unrecognized sentinel");
                }
            }

            #[rustfmt::skip]
            std::println!(r##"{:16}"{}" => {{
                    assert_eq!(val.raw(), {:?});
                }}"##, "", val.name(), val.raw());
        })
        .unwrap();

        assert_eq!(
            CommandData::sentinels(Bitpos(5), |_| {}),
            Err(Error::InvalidField)
        );
    }

    #[test]
    fn device_sentinels() {
        Device::Common
            .sentinels(1, Bitpos(4), |val| {
                match val.name() {
                    "VOUT_COMMAND" => {
                        assert_eq!(val.raw(), 0);
                    }
                    "VOUT_MARGIN_LOW" => {
                        assert_eq!(val.raw(), 1);
                    }
                    "VOUT_MARGIN_HIGH" => {
                        assert_eq!(val.raw(), 2);
                    }
                    "AVS_VOUT_COMMAND" => {
                        assert_eq!(val.raw(), 3);
                    }
                    _ => {
                        panic!("unrecognized sentinel");
                    }
                }

                #[rustfmt::skip]
            std::println!(r##"{:16}"{}" => {{
                    assert_eq!(val.raw(), {:?});
                }}"##, "", val.name(), val.raw());
            })
            .unwrap();

        assert_eq!(
            Device::Common.sentinels(1, Bitpos(5), |_| {}),
            Err(Error::InvalidField)
        );
    }

    #[test]
    fn device_fields() {
        Device::Common
            .fields(1, |f| {
                let bits = f.bits();

                match f.name() {
                    "OnOffState" => {
                        assert_eq!(bits, (Bitpos(7), Bitwidth(1)));
                    }
                    "TurnOffBehavior" => {
                        assert_eq!(bits, (Bitpos(6), Bitwidth(1)));
                    }
                    "VoltageCommandSource" => {
                        assert_eq!(bits, (Bitpos(4), Bitwidth(2)));
                    }
                    "MarginFaultResponse" => {
                        assert_eq!(bits, (Bitpos(2), Bitwidth(2)));
                    }
                    "TransitionControl" => {
                        assert_eq!(bits, (Bitpos(1), Bitwidth(1)));
                    }
                    _ => {
                        panic!("unrecognized field");
                    }
                }

                #[rustfmt::skip]
            std::println!(r##"{:16}"{}" => {{
                    assert_eq!(bits, {:?});
                }}"##, "", f.name(), f.bits());
            })
            .unwrap();
    }

    #[test]
    fn raw() {
        use commands::isl68224::DMAFIX::*;

        let input = [0xef, 0xbe, 0xad, 0xde];

        let mut data = CommandData::from_slice(&input).unwrap();
        assert_eq!(data.get(), Ok(0xdeadbeef));

        let rval = data.mutate(mode, |field, _| {
            assert_eq!(field.bitfield(), false);
            Some(commands::Replacement::Integer(0xbaddcafe))
        });

        assert_eq!(rval, Ok(()));
        assert_eq!(data.0, 0xbaddcafe);

        let rval = data
            .mutate(mode, |_, _| Some(commands::Replacement::Boolean(true)));

        assert_eq!(rval, Err(Error::InvalidReplacement));

        let rval =
            data.mutate(mode, |_, _| Some(commands::Replacement::Float(1.2)));

        assert_eq!(rval, Err(Error::InvalidReplacement));
    }

    #[test]
    fn adm1272_direct() {
        use commands::adm1272::*;
        use units::*;

        let voltage = Coefficients {
            m: 4062,
            b: 0,
            R: -2,
        };
        let current = Coefficients {
            m: 663,
            b: 20480,
            R: -1,
        };
        let power = Coefficients {
            m: 10535,
            b: 0,
            R: -3,
        };

        let vin = READ_VIN::CommandData::from_slice(&[0x6d, 0x07]).unwrap();
        assert_eq!(vin.get(&voltage), Ok(Volts(46.799606)));

        let vin = PEAK_VIN::CommandData::from_slice(&[0x04, 0x09]).unwrap();
        assert_eq!(vin.get(&voltage), Ok(Volts(56.8193)));

        let vout = READ_VOUT::CommandData::from_slice(&[0x51, 0x08]).unwrap();
        assert_eq!(vout.get(&voltage), Ok(Volts(52.412605)));

        let vout = PEAK_VOUT::CommandData::from_slice(&[0x03, 0x09]).unwrap();
        assert_eq!(vout.get(&voltage), Ok(Volts(56.79468)));

        let pin = READ_PIN::CommandData::from_slice(&[0x10, 0x01]).unwrap();
        assert_eq!(pin.get(&power), Ok(Watts(25.818699)));

        let pin = PEAK_PIN::CommandData::from_slice(&[0x3d, 0x01]).unwrap();
        assert_eq!(pin.get(&power), Ok(Watts(30.090176)));

        let iout = READ_IOUT::CommandData::from_slice(&[0x24, 0x08]).unwrap();
        assert_eq!(iout.get(&current), Ok(Amperes(0.54298645)));

        let iout = PEAK_IOUT::CommandData::from_slice(&[0x2b, 0x08]).unwrap();
        assert_eq!(iout.get(&current), Ok(Amperes(0.64856714)));
    }
}
