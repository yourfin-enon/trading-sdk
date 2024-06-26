use crate::{
    calculations::calculate_total_amount,
    positions::{ActivePosition, BidAsk, PendingPosition, Position},
};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use rust_extensions::date_time::DateTimeAsMicroseconds;
use std::{time::Duration};
use rust_extensions::sorted_vec::SortedVec;
use uuid::Uuid;
use crate::assets::{AssetAmount, AssetPrice};
use crate::asset_symbol::AssetSymbol;
use crate::instrument_symbol::InstrumentSymbol;
use crate::position_id::PositionId;
use crate::wallet_id::WalletId;

#[derive(Debug, Clone)]
pub struct Order {
    pub id: String,
    pub trader_id: String,
    pub wallet_id: WalletId,
    pub instrument: InstrumentSymbol,
    pub base_asset: AssetSymbol,
    pub invest_assets: SortedVec<AssetSymbol, AssetAmount>,
    pub leverage: f64,
    pub created_date: DateTimeAsMicroseconds,
    pub side: OrderSide,
    pub take_profit: Option<TakeProfitConfig>,
    pub stop_loss: Option<StopLossConfig>,
    pub stop_out_percent: f64,
    pub margin_call_percent: f64,
    pub top_up_enabled: bool,
    pub top_up_percent: f64,
    pub funding_fee_period: Option<Duration>,
    pub desire_price: Option<f64>,
}

#[derive(Clone, IntoPrimitive, TryFromPrimitive)]
#[repr(i32)]
pub enum OrderType {
    Market = 0,
    Limit = 1,
}

#[derive(Debug, PartialEq, Clone, IntoPrimitive, TryFromPrimitive)]
#[repr(i32)]
pub enum OrderSide {
    Buy = 0,
    Sell = 1,
}

#[derive(Debug, Clone)]
pub struct TakeProfitConfig {
    pub value: f64,
    pub unit: AutoClosePositionUnit,
}

impl TakeProfitConfig {
    pub fn is_triggered(&self, pnl: f64, close_price: f64, side: &OrderSide) -> bool {
        match self.unit {
            AutoClosePositionUnit::AssetAmountUnit => pnl >= self.value,
            AutoClosePositionUnit::PriceRateUnit => match side {
                OrderSide::Buy => self.value <= close_price,
                OrderSide::Sell => self.value >= close_price,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct StopLossConfig {
    pub value: f64,
    pub unit: AutoClosePositionUnit,
}

impl StopLossConfig {
    pub fn is_triggered(&self, pnl: f64, close_price: f64, side: &OrderSide) -> bool {
        match self.unit {
            AutoClosePositionUnit::AssetAmountUnit => pnl < 0.0 && pnl.abs() >= self.value,
            AutoClosePositionUnit::PriceRateUnit => match side {
                OrderSide::Buy => self.value >= close_price,
                OrderSide::Sell => self.value <= close_price,
            },
        }
    }
}

#[derive(Debug, Clone, IntoPrimitive, TryFromPrimitive)]
#[repr(i32)]
pub enum AutoClosePositionUnit {
    AssetAmountUnit = 0,
    PriceRateUnit = 1,
}

impl Order {
    /// returns vec of instruments invested by order
    pub fn get_invest_instruments(&self) -> Vec<InstrumentSymbol> {
        let mut instruments = Vec::with_capacity(self.invest_assets.len());

        for asset in self.invest_assets.iter() {
            let instrument = BidAsk::get_instrument_symbol(&asset.symbol, &self.base_asset);
            instruments.push(instrument);
        }

        instruments
    }

    /// returns vec of all possible instruments
    pub fn get_instruments(&self) -> Vec<InstrumentSymbol> {
        let mut instruments = Vec::with_capacity(self.invest_assets.len() + 1);
        instruments.push(self.instrument.clone());

        for asset in self.invest_assets.iter() {
            let instrument = BidAsk::get_instrument_symbol(&asset.symbol, &self.base_asset);
            instruments.push(instrument);
        }

        instruments
    }

    pub fn get_type(&self) -> OrderType {
        if self.desire_price.is_some() {
            OrderType::Limit
        } else {
            OrderType::Market
        }
    }

    pub fn generate_id() -> String {
        Uuid::new_v4().to_string()
    }

    pub fn validate_prices(&self, asset_prices: &SortedVec<AssetSymbol, AssetPrice>) -> Result<(), String> {
        for item in self.invest_assets.iter() {
            let price = asset_prices.get(&item.symbol);

            if price.is_none() {
                let message = format!("Not Found price for {}", item.symbol);
                return Err(message);
            }
        }

        Ok(())
    }

    pub fn open(self, bidask: &BidAsk, asset_prices: &SortedVec<AssetSymbol, AssetPrice>) -> Position {
        self.open_with_id(Position::generate_id(), bidask, asset_prices)
    }

    pub fn open_with_id(
        self,
        id: PositionId,
        bidask: &BidAsk,
        asset_prices: &SortedVec<AssetSymbol, AssetPrice>,
    ) -> Position {
        if self.validate_prices(asset_prices).is_err() {
            panic!("Can't open order: invalid prices");
        }

        if self.leverage <= 0.0 {
            panic!("Can't open order: leverage can't be less or equals zero");
        }

        match self.get_type() {
            OrderType::Market => {
                let position = self.into_active(id, bidask, asset_prices);
                Position::Active(position)
            }
            OrderType::Limit => {
                let position = self.into_pending(id, bidask, asset_prices);
                position.try_activate()
            }
        }
    }

    pub fn calculate_volume(&self, invest_amount: f64) -> f64 {
        invest_amount * self.leverage
    }

    pub fn calculate_invest_amount(&self, asset_prices: &SortedVec<AssetSymbol, AssetPrice>) -> f64 {
        calculate_total_amount(&self.invest_assets, asset_prices)
    }

    fn into_active(
        self,
        id: PositionId,
        bid_ask: &BidAsk,
        asset_prices: &SortedVec<AssetSymbol, AssetPrice>,
    ) -> ActivePosition {
        let now = DateTimeAsMicroseconds::now();
        let mut asset_prices = asset_prices.to_owned();
        asset_prices.insert_or_replace(AssetPrice {price: 1.0, symbol: self.base_asset.clone()});

        ActivePosition {
            id,
            open_date: now,
            open_price: bid_ask.get_open_price(&self.side),
            open_asset_prices: asset_prices.clone(),
            activate_price: bid_ask.get_open_price(&self.side),
            activate_date: now,
            activate_asset_prices: asset_prices.clone(),
            current_price: bid_ask.get_close_price(&self.side),
            current_asset_prices: asset_prices,
            last_update_date: now,
            top_ups: Vec::new(),
            current_pnl: 0.0,
            current_loss_percent: 0.0,
            prev_loss_percent: 0.0,
            top_up_locked: false,
            total_invest_assets: self.invest_assets.clone(),
            order: self,
            bonus_invest_assets: SortedVec::new_with_capacity(0),
        }
    }

    fn into_pending(
        self,
        id: PositionId,
        bidask: &BidAsk,
        asset_prices: &SortedVec<AssetSymbol, AssetPrice>,
    ) -> PendingPosition {
        let now = DateTimeAsMicroseconds::now();
        let mut asset_prices = asset_prices.to_owned();
        asset_prices.insert_or_replace(AssetPrice {price: 1.0, symbol: self.base_asset.clone()});

        PendingPosition {
            id,
            open_price: bidask.get_open_price(&self.side),
            open_date: now,
            open_asset_prices: asset_prices.clone(),
            current_asset_prices: asset_prices,
            current_price: bidask.get_open_price(&self.side),
            last_update_date: now,
            order: self,
            total_invest_assets: SortedVec::new(),
        }
    }
}
