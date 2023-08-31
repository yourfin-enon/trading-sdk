use crate::top_ups::CanceledTopUp;
use crate::{
    caches::PositionsCache,
    positions::{ActivePosition, BidAsk, ClosedPosition, Position},
};
use ahash::{AHashMap, AHashSet};
use std::time::Duration;

pub struct PositionsMonitor {
    positions_cache: PositionsCache,
    ids_by_instruments: AHashMap<String, AHashSet<String>>,
    cancel_top_up_delay: Duration,
    cancel_top_up_price_change_percent: f64,
    locked_ids: AHashSet<String>,
    pnl_accuracy: Option<u32>,
    pnls_by_wallets: AHashMap<String, AHashMap<String, f64>>,
}

impl PositionsMonitor {
    pub fn new(
        capacity: usize,
        cancel_top_up_delay: Duration,
        cancel_top_up_price_change_percent: f64,
        pnl_accuracy: Option<u32>
    ) -> Self {
        Self {
            pnls_by_wallets: AHashMap::with_capacity(capacity),
            positions_cache: PositionsCache::with_capacity(capacity),
            ids_by_instruments: AHashMap::with_capacity(capacity),
            cancel_top_up_delay,
            locked_ids: AHashSet::with_capacity(capacity),
            cancel_top_up_price_change_percent,
            pnl_accuracy
        }
    }

    pub fn remove(&mut self, position_id: &str) -> Option<Position> {
        if self.locked_ids.contains(position_id) {
            return None;
        }

        let position = self.positions_cache.remove(position_id);

        if let Some(position) = position.as_ref() {
            match position {
                Position::Active(position) => {
                    let pnls_by_instruments =
                        self.pnls_by_wallets.get_mut(&position.order.wallet_id);

                    if let Some(pnls_by_instruments) = pnls_by_instruments {
                        let pnl = pnls_by_instruments.get_mut(&position.order.instrument);

                        if let Some(pnl) = pnl {
                            *pnl -= position.current_pnl;
                        }
                    }
                }
                Position::Closed(_) => {}
                Position::Pending(_) => {}
            }

            self.remove_from_instruments_map(position);
        }

        position
    }

    fn remove_from_instruments_map(&mut self, position: &Position) {
        for instrument in position.get_instruments() {
            if let Some(ids) = self.ids_by_instruments.get_mut(&instrument) {
                ids.remove(position.get_id());
            }
        }
    }

    pub fn add(&mut self, position: Position) {
        self.add_to_instruments_map(&position);
        self.positions_cache.add(position);
    }

    fn add_to_instruments_map(&mut self, position: &Position) {
        let id = position.get_id().to_owned();
        let instruments = position.get_instruments();

        for invest_instrument in instruments {
            if let Some(ids) = self.ids_by_instruments.get_mut(&invest_instrument) {
                ids.insert(id.clone());
            } else {
                self.ids_by_instruments
                    .insert(invest_instrument, AHashSet::from([id.clone()]));
            }
        }
    }

    pub fn get_by_wallet_id(&self, wallet_id: &str) -> Vec<&Position> {
        self.positions_cache.get_by_wallet_id(wallet_id)
    }

    pub fn unlock(&mut self, position: Position) {
        self.locked_ids.remove(position.get_id());
        self.positions_cache.remove(position.get_id());
        self.add(position);
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Position> {
        self.positions_cache.get_mut(id)
    }

    pub fn update(&mut self, bidask: &BidAsk) -> Vec<PositionMonitoringEvent> {
        let position_ids = self.ids_by_instruments.get_mut(&bidask.instrument);

        let Some(position_ids) = position_ids else {
            return Vec::with_capacity(0);
        };

        let mut events = Vec::with_capacity(position_ids.len());
        let mut pnls_by_wallet_id: AHashMap<String, f64> =
            AHashMap::with_capacity(position_ids.len());

        position_ids.retain(|position_id| {
            if self.locked_ids.contains(position_id) {
                // skip update
                return true;
            }

            let position = self.positions_cache.get_mut(position_id);

            let Some(position) = position else {
                return false; // no position in cache so remove id from instruments map
            };

            match position {
                Position::Closed(_) => {
                    let position = match self.positions_cache.remove(position_id).expect("Checked")
                    {
                        Position::Closed(position) => position,
                        _ => panic!("Checked"),
                    };
                    events.push(PositionMonitoringEvent::PositionClosed(position));

                    false // remove closed position
                }
                Position::Pending(position) => {
                    position.update(bidask);

                    if position.can_activate() {
                        let position =
                            match self.positions_cache.remove(position_id).expect("Checked") {
                                Position::Pending(position) => position,
                                _ => panic!("Checked"),
                            };
                        let position = position.into_active();
                        let wallet_pnl = pnls_by_wallet_id.get_mut(&position.order.wallet_id);

                        if let Some(wallet_pnl) = wallet_pnl {
                            *wallet_pnl += position.current_pnl;
                        } else {
                            pnls_by_wallet_id
                                .insert(position.order.wallet_id.clone(), position.current_pnl);
                        }

                        events.push(PositionMonitoringEvent::PositionActivated(position.clone()));
                        self.positions_cache.add(Position::Active(position));
                    }

                    true // active position must be monitored
                }
                Position::Active(position) => {
                    position.update(bidask);

                    if position.is_margin_call() {
                        events.push(PositionMonitoringEvent::PositionMarginCall(
                            position.clone(),
                        ));
                    }

                    if position.is_top_up() {
                        self.locked_ids.insert(position.id.clone());
                        let event = PositionMonitoringEvent::PositionLocked(
                            PositionLockReason::TopUp(position.to_owned()),
                        );
                        events.push(event);
                    } else {
                        let canceled_top_ups = position.try_cancel_top_ups(
                            self.cancel_top_up_price_change_percent,
                            self.cancel_top_up_delay,
                        );

                        if !canceled_top_ups.is_empty() {
                            self.locked_ids.insert(position.id.clone());
                            let reason = PositionLockReason::TopUpsCanceled((
                                position.to_owned(),
                                canceled_top_ups,
                            ));
                            let event = PositionMonitoringEvent::PositionLocked(reason);
                            events.push(event);
                        }
                    }

                    if let Some(reason) = position.determine_close_reason() {
                        let position = match self
                            .positions_cache
                            .remove(position_id)
                            .expect("Must exists")
                        {
                            Position::Active(position) => position,
                            _ => panic!("Position is in Active case"),
                        };
                        let position = position.close(reason, self.pnl_accuracy);
                        events.push(PositionMonitoringEvent::PositionClosed(position));

                        false // remove closed position
                    } else {
                        let wallet_pnl = pnls_by_wallet_id.get_mut(&position.order.wallet_id);

                        if let Some(wallet_pnl) = wallet_pnl {
                            *wallet_pnl += position.current_pnl;
                        } else {
                            pnls_by_wallet_id
                                .insert(position.order.wallet_id.clone(), position.current_pnl);
                        }

                        true // no need to do anything with position
                    }
                }
            }
        });

        for (wallet_id, pnl) in pnls_by_wallet_id {
            let pnls_by_instruments = self.pnls_by_wallets.get_mut(&wallet_id);

            if let Some(pnls_by_instruments) = pnls_by_instruments {
                pnls_by_instruments.insert(bidask.instrument.clone(), pnl);
            } else {
                let pnl = AHashMap::from([(bidask.instrument.clone(), pnl)]);
                self.pnls_by_wallets.insert(wallet_id, pnl);
            }
        }

        events
    }
}

pub enum PositionMonitoringEvent {
    PositionClosed(ClosedPosition),
    PositionActivated(ActivePosition),
    PositionMarginCall(ActivePosition),
    PositionLocked(PositionLockReason),
}

pub enum PositionLockReason {
    TopUp(ActivePosition),
    TopUpsCanceled((ActivePosition, Vec<CanceledTopUp>)),
}

#[cfg(test)]
mod tests {}
