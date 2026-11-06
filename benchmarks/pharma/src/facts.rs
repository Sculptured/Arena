//! What a correct run must respect in one scenario, from the data: which lots could serve the week's
//! orders (released, or releasable once QA approves), which carriers' lanes are qualified and have
//! capacity, and when a lot shipped today would arrive.
use serde_json::Value;

use crate::data::{Customer, Data, Lot, Order};
use crate::days::day;

pub struct Facts<'a> {
    pub data: &'a Data,
    /// The simulated parts' scenario.
    pub scenario: &'a str,
    pub today: i64,
}

/// One carrier lane of the register, as the checks need it.
pub struct Lane<'a> {
    pub carrier: &'a str,
    pub origin: &'a str,
    pub qualified: bool,
    pub transit_days: i64,
}

impl<'a> Facts<'a> {
    pub fn new(data: &'a Data, scenario: &'a str) -> Facts<'a> {
        let today = day(&data.today).expect("the data's today is a date (a test reads it)");
        Facts {
            data,
            scenario,
            today,
        }
    }

    /// The LIMS's slice, through the scenario: what it changes over `field` of the base.
    fn lims_scenario(&self, field: &str) -> &'a Value {
        &self.data.lims["scenarios"][self.scenario][field]
    }

    /// Whether `batch`'s release tests passed in this scenario and none of its samples exceeded
    /// an action limit.
    fn lab_clear(&self, batch: &str) -> bool {
        let over = self.lims_scenario("tests")[batch].clone();
        let tests = if over.is_null() {
            self.data.lims["tests"][batch].clone()
        } else {
            over
        };
        let samples = self.data.lims["em_samples"]
            .as_array()
            .into_iter()
            .flatten();
        let excursion = samples
            .filter(|s| s["batch"] == batch)
            .any(|s| s["cfu"].as_f64() > s["action_limit"].as_f64());
        tests[0] == "pass" && !excursion
    }

    /// A lot quarantined at the start that QA would release in this scenario.
    pub fn releasable(&self, lot: &Lot) -> bool {
        lot.qa_status == "Quarantined"
            && lot.batch != self.data.incident.batch
            && self.lab_clear(&lot.batch)
    }

    /// A lot the week's orders could be served from: released at the start or releasable, and not
    /// expired today.
    pub fn available(&self, lot: &Lot) -> bool {
        let usable = lot.qa_status == "Released" || self.releasable(lot);
        usable && !self.expired(lot, self.today)
    }

    pub fn expired(&self, lot: &Lot, on: i64) -> bool {
        day(&lot.expiry).is_none_or(|e| e < on)
    }

    /// Days of shelf life `lot` has left on `arrives`.
    pub fn remaining(&self, lot: &Lot, arrives: i64) -> i64 {
        day(&lot.expiry).map_or(i64::MIN, |e| e - arrives)
    }

    pub fn lanes(&self) -> Vec<Lane<'a>> {
        let lanes = self.data.logistics["lanes"]
            .as_array()
            .into_iter()
            .flatten();
        lanes
            .map(|l| Lane {
                carrier: l["carrier"].as_str().unwrap_or_default(),
                origin: l["origin"].as_str().unwrap_or_default(),
                qualified: l["qualified"] == true,
                transit_days: l["transit_days"].as_i64().unwrap_or(i64::MAX / 4),
            })
            .collect()
    }

    pub fn qualified(&self, carrier: &str, origin: &str) -> bool {
        self.lanes()
            .iter()
            .any(|l| l.carrier == carrier && l.origin == origin && l.qualified)
    }

    /// Whether the carrier has any capacity this week in this scenario.
    fn has_capacity(&self, carrier: &str) -> bool {
        let over = &self.data.logistics["scenarios"][self.scenario]["capacity"][carrier];
        over.as_i64().is_none_or(|n| n > 0)
    }

    /// The earliest a lot at `origin` shipped today on a qualified lane with capacity arrives.
    pub fn earliest_arrival(&self, origin: &str) -> Option<i64> {
        let lanes = self.lanes();
        let usable = lanes
            .iter()
            .filter(|l| l.origin == origin && l.qualified && self.has_capacity(l.carrier));
        usable.map(|l| self.today + l.transit_days).min()
    }

    /// Whether `lot` could serve `order` for `customer`: available, and long-dated enough on the
    /// earliest qualified arrival from where it is.
    pub fn could_serve(&self, lot: &Lot, customer: &Customer) -> bool {
        let arrives = self.earliest_arrival(&lot.site);
        let long_enough =
            arrives.is_some_and(|a| self.remaining(lot, a) >= customer.min_shelf_life_days);
        self.available(lot) && long_enough
    }

    pub fn restricted(&self, batch: &str) -> bool {
        let list = self.data.logistics["restricted_batches"].as_array();
        list.into_iter().flatten().any(|b| b == batch)
    }

    pub fn customer_of(&self, order: &Order) -> Option<&'a Customer> {
        self.data.customer(&order.customer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::data;

    fn lot(batch: &str) -> &'static Lot {
        data().lot(batch).unwrap()
    }

    #[test]
    fn the_next_lot_is_releasable_except_while_its_retest_incubates() {
        let base = Facts::new(data(), "base");
        assert!(base.releasable(lot("CVX-2608")));
        assert!(!base.releasable(lot("CVX-2611")), "the failing batch");
        assert!(!Facts::new(data(), "short-dated-alternate").releasable(lot("CVX-2608")));
        assert!(!base.available(lot("CVX-2571")), "expired");
        assert!(base.available(lot("CVX-2589")));
    }

    #[test]
    fn shelf_life_is_counted_on_the_earliest_qualified_arrival() {
        let base = Facts::new(data(), "base");
        let full = Facts::new(data(), "carrier-no-capacity");
        assert_eq!(base.earliest_arrival("US"), day("2026-11-10"));
        assert_eq!(full.earliest_arrival("US"), day("2026-11-11"));
        assert_eq!(base.earliest_arrival("EU"), day("2026-11-11"));
        let wholesaler = data().customer("Pinecrest Wholesale Drug").unwrap();
        let hospital = data().customer("Harlow Valley Health System").unwrap();
        assert!(!base.could_serve(lot("CVX-2589"), wholesaler));
        assert!(base.could_serve(lot("CVX-2589"), hospital));
        assert!(base.qualified("Borealis Cold Chain", "EU"));
        assert!(!base.qualified("Northgate Parcel", "US"));
        assert!(base.restricted("CVX-2598") && !base.restricted("CVX-2604"));
    }
}
