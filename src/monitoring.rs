use crate::asset_symbol::AssetSymbol;
use crate::assets::AssetAmount;
use crate::instrument_symbol::InstrumentSymbol;
use crate::position_id::PositionId;
use crate::positions::PendingPosition;
use crate::top_ups::{ActiveTopUp, CanceledTopUp};
use crate::wallet_id::WalletId;
use crate::wallets::{Wallet, WalletBalance};
use crate::{
    caches::PositionsCache,
    positions::{ActivePosition, BidAsk, ClosedPosition, Position},
};
use ahash::{AHashMap, AHashSet};
use rust_extensions::sorted_vec::{EntityWithKey, SortedVec};
use std::time::Duration;

pub struct PositionIdsByInstrumentSymbol {
    pub items: AHashSet<PositionId>,
    instrument_symbol: InstrumentSymbol,
}

impl PositionIdsByInstrumentSymbol {
    pub fn new(instrument_symbol: InstrumentSymbol) -> Self {
        PositionIdsByInstrumentSymbol {
            items: Default::default(),
            instrument_symbol,
        }
    }

    pub fn new_with_one(instrument_symbol: InstrumentSymbol, id: PositionId) -> Self {
        PositionIdsByInstrumentSymbol {
            items: AHashSet::from([id]),
            instrument_symbol,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl EntityWithKey<InstrumentSymbol> for PositionIdsByInstrumentSymbol {
    fn get_key(&self) -> &InstrumentSymbol {
        &self.instrument_symbol
    }
}

pub struct WalletIdsByInstrumentSymbol {
    pub items: AHashSet<WalletId>,
    instrument_symbol: InstrumentSymbol,
}

impl WalletIdsByInstrumentSymbol {
    pub fn new(instrument_symbol: InstrumentSymbol) -> Self {
        WalletIdsByInstrumentSymbol {
            items: Default::default(),
            instrument_symbol,
        }
    }

    pub fn new_with_one(instrument_symbol: InstrumentSymbol, id: WalletId) -> Self {
        WalletIdsByInstrumentSymbol {
            items: AHashSet::from([id]),
            instrument_symbol,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl EntityWithKey<InstrumentSymbol> for WalletIdsByInstrumentSymbol {
    fn get_key(&self) -> &InstrumentSymbol {
        &self.instrument_symbol
    }
}

pub struct PositionsMonitor {
    positions_cache: PositionsCache,
    ids_by_instruments: SortedVec<InstrumentSymbol, PositionIdsByInstrumentSymbol>,
    cancel_top_up_delay: Duration,
    cancel_top_up_price_change_percent: f64,
    locked_ids: SortedVec<PositionId, PositionId>,
    pnl_accuracy: Option<u32>,
    wallets_by_ids: AHashMap<WalletId, Wallet>,
    wallet_ids_by_instruments: SortedVec<InstrumentSymbol, WalletIdsByInstrumentSymbol>,
    wallet_monitoring_enabled: bool,
    last_update_events_count: usize,
    // reused allocations
    top_up_pnls_by_wallet_ids: AHashMap<WalletId, f64>,
    top_up_reserved_by_wallet_ids: AHashMap<WalletId, SortedVec<AssetSymbol, AssetAmount>>,
}

impl PositionsMonitor {
    pub fn new(
        capacity: usize,
        cancel_top_up_delay: Duration,
        cancel_top_up_price_change_percent: f64,
        pnl_accuracy: Option<u32>,
        wallet_monitoring_enabled: bool,
    ) -> Self {
        let instruments_count = 500;
        let wallet_ids_count = capacity / 20;

        Self {
            wallets_by_ids: AHashMap::with_capacity(wallet_ids_count),
            positions_cache: PositionsCache::with_capacity(capacity),
            ids_by_instruments: SortedVec::new_with_capacity(instruments_count),
            cancel_top_up_delay,
            locked_ids: SortedVec::new_with_capacity(capacity / 1000),
            cancel_top_up_price_change_percent,
            pnl_accuracy,
            wallet_ids_by_instruments: SortedVec::new_with_capacity(instruments_count),
            top_up_pnls_by_wallet_ids: AHashMap::with_capacity(wallet_ids_count),
            top_up_reserved_by_wallet_ids: AHashMap::with_capacity(wallet_ids_count),
            wallet_monitoring_enabled,
            last_update_events_count: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.positions_cache.count()
    }

    pub fn get_wallet_mut(&mut self, wallet_id: &WalletId) -> Option<&mut Wallet> {
        let wallet = self.wallets_by_ids.get_mut(wallet_id);

        if let Some(wallet) = wallet {
            return Some(wallet);
        }

        None
    }

    pub fn contains_wallet(&self, wallet_id: &WalletId) -> bool {
        self.wallets_by_ids.contains_key(wallet_id)
    }

    pub fn remove(&mut self, position_id: &PositionId) -> Option<Position> {
        if self.locked_ids.contains(position_id) {
            return None;
        }

        let position = self.positions_cache.remove(position_id);

        if let Some(position) = position.as_ref() {
            match position {
                Position::Active(position) => {
                    if position.order.top_up_enabled
                        && self
                            .positions_cache
                            .contains_by_wallet_id(&position.order.wallet_id)
                    {
                        let wallet = self.wallets_by_ids.get_mut(&position.order.wallet_id);

                        if let Some(wallet) = wallet {
                            wallet.deduct_top_up_pnl(
                                &position.order.instrument,
                                position.current_pnl,
                            );
                        }
                    } else {
                        self.remove_wallet(&position.order.wallet_id);
                    }
                }
                Position::Closed(_) => {}
                Position::Pending(_) => {}
            }

            for instrument in position.get_instruments() {
                if let Some(ids) = self.ids_by_instruments.get_mut(&instrument) {
                    ids.items.remove(position.get_id());
                }
            }
        }

        position
    }

    pub fn remove_wallet(&mut self, wallet_id: &WalletId) -> Option<Wallet> {
        let wallet = self.wallets_by_ids.remove(wallet_id);

        if let Some(wallet) = wallet {
            for instrument in wallet.get_instruments() {
                let wallet_ids = self.wallet_ids_by_instruments.get_mut(instrument);

                if let Some(wallet_ids) = wallet_ids {
                    wallet_ids.items.remove(wallet_id);
                }
            }

            return Some(wallet);
        }

        None
    }

    pub fn add_wallet(&mut self, wallet: Wallet) {
        for instrument in wallet.get_instruments() {
            let wallet_ids = self.wallet_ids_by_instruments.get_mut(instrument);

            if let Some(wallet_ids) = wallet_ids {
                wallet_ids.items.insert(wallet.id.clone());
            } else {
                self.wallet_ids_by_instruments.insert_or_replace(
                    WalletIdsByInstrumentSymbol::new_with_one(
                        instrument.clone(),
                        wallet.id.clone(),
                    ),
                );
            }
        }

        self.wallets_by_ids.insert(wallet.id.clone(), wallet);
    }

    pub fn update_wallet(
        &mut self,
        wallet_id: &WalletId,
        balance: WalletBalance,
    ) -> Result<Option<Wallet>, String> {
        let wallet = self.wallets_by_ids.get_mut(wallet_id);

        let Some(wallet) = wallet else {
            return Ok(None);
        };

        wallet.update_balance(balance)?;

        Ok(Some(wallet.to_owned()))
    }

    pub fn add(&mut self, position: Position) {
        let id = position.get_id().to_owned();
        let instruments = position.get_instruments();

        for invest_instrument in instruments {
            if let Some(ids) = self.ids_by_instruments.get_mut(&invest_instrument) {
                ids.items.insert(id.clone());
            } else {
                self.ids_by_instruments.insert_or_replace(
                    PositionIdsByInstrumentSymbol::new_with_one(invest_instrument, id.clone()),
                );
            }
        }

        self.positions_cache.add(position);
    }

    pub fn get_by_wallet_id(&self, wallet_id: &WalletId, limit: usize) -> Vec<&Position> {
        self.positions_cache.get_by_wallet_id(wallet_id, limit)
    }

    pub fn unlock(&mut self, position_id: &PositionId) {
        self.locked_ids.remove(position_id);
    }

    pub fn add_top_up(
        &mut self,
        position: &ActivePosition,
        top_up: ActiveTopUp,
    ) -> Result<(), String> {
        let position = self.positions_cache.get_mut(&position.id);

        let Some(position) = position else {
            return Err("Position not found".to_string());
        };

        match position {
            Position::Active(position) => {
                position.add_top_up(top_up);
                Ok(())
            }
            Position::Closed(_) => Err("Can't add top-up to closed position ".to_string()),
            Position::Pending(_) => Err("Can't add top-up to pending position".to_string()),
        }
    }

    pub fn get_mut(&mut self, id: &PositionId) -> Option<&mut Position> {
        self.positions_cache.get_mut(id)
    }

    fn clear_reused_allocations(&mut self) {
        self.top_up_pnls_by_wallet_ids.clear();
        self.top_up_reserved_by_wallet_ids.clear();
    }

    pub fn update(&mut self, bidask: &BidAsk) -> Vec<PositionMonitoringEvent> {
        let position_ids = self.ids_by_instruments.get_mut(&bidask.instrument);

        let Some(position_ids) = position_ids else {
            return Vec::with_capacity(0);
        };

        let mut events = Vec::with_capacity(self.last_update_events_count / 4 + 10);
        let wallet_ids_to_remove_count = if self.wallet_monitoring_enabled { self.wallets_by_ids.len() / 1000 + 10 } else { 0 };
        let mut wallet_ids_to_remove = Vec::with_capacity(wallet_ids_to_remove_count);

        position_ids.items.retain(|position_id| {
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

                    if position.is_price_reached() {
                        if position.can_activate() {
                            let position =
                                match self.positions_cache.remove(position_id).expect("Checked") {
                                    Position::Pending(position) => position,
                                    _ => panic!("Checked"),
                                };
                            let mut position =
                                position.activate().expect("checked by can_activate");
                            position.update(bidask);
                            events
                                .push(PositionMonitoringEvent::PositionActivated(position.clone()));
                            self.positions_cache.add(Position::Active(position));
                        } else {
                            self.locked_ids.insert_or_replace(position.id.clone());
                            let lock_reason =
                                PositionLockReason::ActivationPending(position.clone());
                            events.push(PositionMonitoringEvent::PositionLocked(lock_reason));
                        }
                    }

                    true // pending position must be monitored
                }
                Position::Active(position) => {
                    position.update(bidask);

                    if position.is_margin_call() {
                        events.push(PositionMonitoringEvent::PositionMarginCall(
                            position.clone(),
                        ));
                    }

                    if position.is_top_up() {
                        self.locked_ids.insert_or_replace(position.id.clone());
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
                            self.locked_ids.insert_or_replace(position.id.clone());
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

                        if self.wallet_monitoring_enabled && self
                            .positions_cache
                            .contains_by_wallet_id(&position.order.wallet_id)
                        {
                            wallet_ids_to_remove.push(position.order.wallet_id.clone());
                        }

                        events.push(PositionMonitoringEvent::PositionClosed(position));

                        false // remove closed position
                    } else {
                        if position.order.top_up_enabled {
                            let wallet_pnl = self
                                .top_up_pnls_by_wallet_ids
                                .get_mut(&position.order.wallet_id);

                            if let Some(wallet_pnl) = wallet_pnl {
                                *wallet_pnl += position.current_pnl;
                            } else {
                                self.top_up_pnls_by_wallet_ids
                                    .insert(position.order.wallet_id.clone(), position.current_pnl);
                            }

                            // calc reserved amounts
                            let reserved_by_assets = self
                                .top_up_reserved_by_wallet_ids
                                .get_mut(&position.order.wallet_id);

                            if let Some(reserved_by_assets) = reserved_by_assets {
                                for item in position.total_invest_assets.iter() {
                                    let reserved_amount = reserved_by_assets.get_mut(&item.symbol);

                                    if let Some(reserved_amount) = reserved_amount {
                                        reserved_amount.amount += item.amount;
                                    } else {
                                        reserved_by_assets.insert_or_replace(AssetAmount {
                                            amount: item.amount,
                                            symbol: item.symbol.clone(),
                                        });
                                    }
                                }
                            } else {
                                self.top_up_reserved_by_wallet_ids.insert(
                                    position.order.wallet_id.clone(),
                                    position.order.invest_assets.clone(),
                                );
                            }
                        }

                        true // no need to do anything with position
                    }
                }
            }
        });

        if self.wallet_monitoring_enabled {
            for wallet_id in wallet_ids_to_remove {
                self.remove_wallet(&wallet_id);
            }

            self.update_wallet_prices(bidask);
            self.update_wallet_reserved(bidask);
            for event in self.update_wallet_pnls(bidask) {
                events.push(event);
            }
        }
        
        self.clear_reused_allocations();
        self.last_update_events_count = events.len();

        events
    }

    fn update_wallet_prices(&mut self, bidask: &BidAsk) {
        let wallet_ids = self.wallet_ids_by_instruments.get_mut(&bidask.instrument);

        if let Some(wallet_ids) = wallet_ids {
            for wallet_id in wallet_ids.items.iter() {
                let wallet = self
                    .wallets_by_ids
                    .get_mut(wallet_id)
                    .expect("invalid wallet add");
                wallet.update_price(bidask);
            }
        }
    }

    fn update_wallet_reserved(&mut self, bidask: &BidAsk) {
        for (wallet_id, reserved_by_assets) in &self.top_up_reserved_by_wallet_ids {
            let wallet = self.wallets_by_ids.get_mut(wallet_id);

            let Some(wallet) = wallet else {
                continue;
            };

            wallet.set_top_up_reserved(&bidask.instrument, reserved_by_assets);
        }
    }

    fn update_wallet_pnls(&mut self, bidask: &BidAsk) -> Vec<PositionMonitoringEvent> {
        let mut events = Vec::new();

        for (wallet_id, pnl) in self.top_up_pnls_by_wallet_ids.iter() {
            let wallet = self.wallets_by_ids.get_mut(&wallet_id);

            let Some(wallet) = wallet else {
                continue;
            };

            wallet.set_top_up_pnl(&bidask.instrument, *pnl);
            wallet.update_loss();

            if wallet.is_margin_call() {
                events.push(PositionMonitoringEvent::WalletMarginCall(
                    WalletMarginCallInfo {
                        loss_percent: wallet.current_loss_percent,
                        pnl: *pnl,
                        wallet_id: wallet.id.clone(),
                        trader_id: wallet.trader_id.clone(),
                    },
                ));
            }
        }

        events
    }
}

pub enum PositionMonitoringEvent {
    /// Active position was closed due to stop-out and removed from cache
    PositionClosed(ClosedPosition),
    /// Pending position with already reserved assets was activated due to price
    /// and re-added as active position to cache
    PositionActivated(ActivePosition),
    /// Active position has margin call
    PositionMarginCall(ActivePosition),
    /// Active position was locked with inner reason
    PositionLocked(PositionLockReason),
    /// Wallet has margin call
    WalletMarginCall(WalletMarginCallInfo),
}

pub enum PositionLockReason {
    /// Active position needs to add a top-up
    TopUp(ActivePosition),
    /// Active position needs to cancel the top-ups
    TopUpsCanceled((ActivePosition, Vec<CanceledTopUp>)),
    /// Pending position without reserved assets reached desire price needs to reserve assets
    ActivationPending(PendingPosition),
}

#[derive(Debug)]
pub struct WalletMarginCallInfo {
    pub loss_percent: f64,
    pub pnl: f64,
    pub wallet_id: WalletId,
    pub trader_id: String,
}

#[cfg(test)]
mod tests {}
