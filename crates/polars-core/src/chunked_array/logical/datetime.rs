use super::*;
use crate::datatypes::time_unit::TimeUnit;
use crate::prelude::*;

pub type DatetimeChunked = Logical<DatetimeType, Int64Type>;

impl Int64Chunked {
    pub fn into_datetime(self, timeunit: TimeUnit, tz: Option<TimeZone>) -> DatetimeChunked {
        // SAFETY: no invalid states.
        unsafe { DatetimeChunked::new_logical(self, DataType::Datetime(timeunit, tz)) }
    }
}

impl LogicalType for DatetimeChunked {
    fn dtype(&self) -> &DataType {
        &self.dtype
    }

    fn get_any_value(&self, i: usize) -> PolarsResult<AnyValue<'_>> {
        self.phys
            .get_any_value(i)
            .map(|av| av.as_datetime(self.time_unit(), self.time_zone().as_ref()))
    }

    unsafe fn get_any_value_unchecked(&self, i: usize) -> AnyValue<'_> {
        self.phys
            .get_any_value_unchecked(i)
            .as_datetime(self.time_unit(), self.time_zone().as_ref())
    }

    fn cast_with_options(
        &self,
        dtype: &DataType,
        cast_options: CastOptions,
    ) -> PolarsResult<Series> {
        use DataType::*;

        use crate::datatypes::time_unit::TimeUnit::*;

        let out = match dtype {
            Datetime(to_unit, tz) => {
                let from_unit = self.time_unit();
                let (multiplier, divisor) = match (from_unit, to_unit) {
                    // scaling from lower precision to higher precision
                    (Milliseconds, Nanoseconds) => (Some(1_000_000i64), None),
                    (Milliseconds, Microseconds) => (Some(1_000i64), None),
                    (Microseconds, Nanoseconds) => (Some(1_000i64), None),
                    // scaling from higher precision to lower precision
                    (Nanoseconds, Milliseconds) => (None, Some(1_000_000i64)),
                    (Nanoseconds, Microseconds) => (None, Some(1_000i64)),
                    (Microseconds, Milliseconds) => (None, Some(1_000i64)),
                    _ => return self.phys.cast_with_options(dtype, cast_options),
                };
                match multiplier {
                    // scale to higher precision (eg: ms → us, ms → ns, us → ns)
                    Some(m) => Ok((self.phys.as_ref().checked_mul_scalar(m))
                        .into_datetime(*to_unit, tz.clone())
                        .into_series()),
                    // scale to lower precision (eg: ns → us, ns → ms, us → ms)
                    None => match divisor {
                        Some(d) => Ok(self
                            .phys
                            .apply_values(|v| v.div_euclid(d))
                            .into_datetime(*to_unit, tz.clone())
                            .into_series()),
                        None => unreachable!("must always have a time unit divisor here"),
                    },
                }
            },
            #[cfg(feature = "dtype-date")]
            Date => {
                let cast_to_date = |tu_in_day: i64| {
                    let mut dt = self
                        .phys
                        .apply_values(|v| v.div_euclid(tu_in_day))
                        .cast_with_options(&Int32, cast_options)
                        .unwrap()
                        .into_date()
                        .into_series();
                    dt.set_sorted_flag(self.physical().is_sorted_flag());
                    Ok(dt)
                };
                match self.time_unit() {
                    Nanoseconds => cast_to_date(NS_IN_DAY),
                    Microseconds => cast_to_date(US_IN_DAY),
                    Milliseconds => cast_to_date(MS_IN_DAY),
                }
            },
            #[cfg(feature = "dtype-time")]
            Time => {
                let (scaled_mod, multiplier) = match self.time_unit() {
                    Nanoseconds => (NS_IN_DAY, 1i64),
                    Microseconds => (US_IN_DAY, 1_000i64),
                    Milliseconds => (MS_IN_DAY, 1_000_000i64),
                };
                return Ok(self
                    .phys
                    .apply(|v| {
                        let t = (v? % scaled_mod).checked_mul(multiplier)?;
                        t.checked_add(NS_IN_DAY * (t < 0) as i64)
                    })
                    .into_time()
                    .into_series());
            },
            dt if dt.is_primitive_numeric() => {
                return self.phys.cast_with_options(dtype, cast_options);
            },
            dt => {
                polars_bail!(
                    InvalidOperation:
                    "casting from {:?} to {:?} not supported",
                    self.dtype(), dt
                )
            },
        };
        out.map(|mut s| {
            // TODO!; implement the divisions/multipliers above
            // in a checked manner so that we raise on overflow
            s.set_sorted_flag(self.physical().is_sorted_flag());
            s
        })
    }
}
