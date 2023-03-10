use crate::{
    calculations::{calculate_margin_percent, calculate_total_amount},
    orders::{Order, OrderSide, StopLossConfig, TakeProfitConfig},
};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use rust_extensions::date_time::DateTimeAsMicroseconds;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, IntoPrimitive, TryFromPrimitive)]
#[repr(i32)]
pub enum ClosePositionReason {
    ClientCommand = 0,
    StopOut = 1,
    TakeProfit = 2,
    StopLoss = 3,
    AdminCommand = 4,
}

#[derive(Clone)]
pub struct BidAsk {
    pub instrument: String,
    pub datetime: DateTimeAsMicroseconds,
    pub bid: f64,
    pub ask: f64,
}

impl BidAsk {
    pub fn generate_id(base_asset: &str, quote_asset: &str) -> String {
        let id = format!("{}{}", base_asset, quote_asset); // todo: find better solution

        id
    }

    pub fn get_close_price(&self, side: &OrderSide) -> f64 {
        match side {
            OrderSide::Buy => self.bid,
            OrderSide::Sell => self.ask,
        }
    }

    pub fn get_open_price(&self, side: &OrderSide) -> f64 {
        match side {
            OrderSide::Buy => self.ask,
            OrderSide::Sell => self.bid,
        }
    }

    pub fn get_asset_price(&self, asset: &str, side: &OrderSide) -> f64 {
        match side {
            OrderSide::Sell => {
                if self.instrument.starts_with(asset) {
                    self.ask
                } else {
                    panic!("Invalid instrument {} for asset {}", self.instrument, asset)
                }
            }
            OrderSide::Buy => {
                if self.instrument.starts_with(asset) {
                    self.bid
                } else {
                    panic!("Invalid instrument {} for asset {}", self.instrument, asset)
                }
            }
        }
    }
}

#[derive(Clone)]
pub enum Position {
    Active(ActivePosition),
    Closed(ClosedPosition),
    Pending(PendingPosition),
}

impl Position {
    pub fn generate_id() -> String {
        Uuid::new_v4().to_string()
    }

    pub fn get_id(&self) -> &str {
        match self {
            Position::Active(position) => &position.id,
            Position::Closed(position) => &position.id,
            Position::Pending(position) => &position.id,
        }
    }

    pub fn get_open_asset_prices(&self) -> &HashMap<String, f64> {
        match self {
            Position::Active(position) => &position.open_asset_prices,
            Position::Closed(position) => &position.open_asset_prices,
            Position::Pending(position) => &position.open_asset_prices,
        }
    }

    pub fn get_open_date(&self) -> DateTimeAsMicroseconds {
        match self {
            Position::Active(position) => position.open_date,
            Position::Closed(position) => position.open_date,
            Position::Pending(position) => position.open_date,
        }
    }

    pub fn get_order(&self) -> &Order {
        match self {
            Position::Active(position) => &position.order,
            Position::Closed(position) => &position.order,
            Position::Pending(position) => &position.order,
        }
    }

    pub fn get_status(&self) -> PositionStatus {
        match self {
            Position::Pending(_position) => PositionStatus::Pending,
            Position::Active(_position) => PositionStatus::Active,
            Position::Closed(position) => position.get_status(),
        }
    }
}

#[derive(Clone, IntoPrimitive, TryFromPrimitive, PartialEq)]
#[repr(i32)]
pub enum PositionStatus {
    Pending = 0,
    Active = 1,
    Filled = 2,
    Canceled = 3,
}

#[derive(Clone)]
pub struct PendingPosition {
    pub id: String,
    pub order: Order,
    pub open_date: DateTimeAsMicroseconds,
    pub open_asset_prices: HashMap<String, f64>,
    pub current_price: f64,
    pub current_asset_prices: HashMap<String, f64>,
    pub last_update_date: DateTimeAsMicroseconds,
}

impl PendingPosition {
    pub fn update(&mut self, bidask: &BidAsk) {
        self.try_update_price(bidask);
        self.try_update_asset_price(bidask);
        self.last_update_date = DateTimeAsMicroseconds::now();
    }

    pub fn can_activate(&self) -> bool {
        let Some(desired_price) = self.order.desire_price else {
            panic!("PendingPosition without desire price");
        };

        self.current_price >= desired_price && self.order.side == OrderSide::Sell
            || self.current_price <= desired_price && self.order.side == OrderSide::Buy
    }

    fn try_update_price(&mut self, bidask: &BidAsk) {
        if self.order.instrument == bidask.instrument {
            self.current_price = bidask.get_open_price(&self.order.side)
        }
    }

    fn try_update_asset_price(&mut self, bidask: &BidAsk) {
        for asset in self.order.invest_assets.keys() {
            let id = BidAsk::generate_id(asset, &self.order.base_asset);

            if id == bidask.instrument {
                let price = bidask.get_asset_price(asset, &OrderSide::Sell);
                let current_asset_price = self.current_asset_prices.get_mut(asset);

                if let Some(current_asset_price) = current_asset_price {
                    *current_asset_price = price;
                } else {
                    self.current_asset_prices.insert(asset.to_owned(), price);
                }
            }
        }
    }

    pub fn try_activate(self) -> Position {
        if self.can_activate() {
            return Position::Active(self.into_active());
        }

        Position::Pending(self)
    }

    pub fn into_active(self) -> ActivePosition {
        if !self.can_activate() {
            panic!("Can't activate");
        }

        let now = DateTimeAsMicroseconds::now();

        ActivePosition {
            id: self.id,
            open_date: self.open_date,
            open_asset_prices: self.open_asset_prices,
            activate_price: self.current_price,
            activate_date: now,
            activate_asset_prices: self.current_asset_prices.to_owned(),
            order: self.order,
            current_price: self.current_price,
            current_asset_prices: self.current_asset_prices,
            last_update_date: now,
        }
    }

    pub fn set_take_profit(&mut self, value: Option<TakeProfitConfig>) {
        self.order.take_profit = value;
    }

    pub fn set_stop_loss(&mut self, value: Option<StopLossConfig>) {
        self.order.stop_loss = value;
    }

    pub fn set_desire_price(&mut self, value: f64) {
        self.order.desire_price = Some(value);
    }

    pub fn close(self, reason: ClosePositionReason) -> ClosedPosition {
        ClosedPosition {
            pnl: None,
            asset_pnls: HashMap::new(),
            open_date: self.open_date,
            open_asset_prices: self.open_asset_prices,
            activate_date: None,
            activate_price: None,
            activate_asset_prices: HashMap::new(),
            close_date: DateTimeAsMicroseconds::now(),
            close_price: self.current_price,
            close_reason: reason,
            close_asset_prices: self.current_asset_prices.to_owned(),
            order: self.order,
            id: self.id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivePosition {
    pub id: String,
    pub order: Order,
    pub open_date: DateTimeAsMicroseconds,
    pub open_asset_prices: HashMap<String, f64>,
    pub activate_price: f64,
    pub activate_date: DateTimeAsMicroseconds,
    pub activate_asset_prices: HashMap<String, f64>,
    pub current_price: f64,
    pub current_asset_prices: HashMap<String, f64>,
    pub last_update_date: DateTimeAsMicroseconds,
}

impl ActivePosition {
    pub fn set_take_profit(&mut self, value: Option<TakeProfitConfig>) {
        self.order.take_profit = value;
    }

    pub fn set_stop_loss(&mut self, value: Option<StopLossConfig>) {
        self.order.stop_loss = value;
    }

    pub fn update(&mut self, bidask: &BidAsk) {
        self.try_update_price(bidask);
        self.try_update_asset_price(bidask);
    }

    fn try_update_price(&mut self, bidask: &BidAsk) {
        if self.order.instrument == bidask.instrument {
            self.current_price = bidask.get_close_price(&self.order.side)
        }
    }

    fn try_update_asset_price(&mut self, bidask: &BidAsk) {
        for asset in self.order.invest_assets.keys() {
            let id = BidAsk::generate_id(asset, &self.order.base_asset);

            if id == bidask.instrument {
                let price = bidask.get_asset_price(asset, &OrderSide::Sell);
                let current_asset_price = self.current_asset_prices.get_mut(asset);

                if let Some(current_asset_price) = current_asset_price {
                    *current_asset_price = price;
                } else {
                    self.current_asset_prices.insert(asset.to_owned(), price);
                }
            }
        }
    }

    pub fn close(self, reason: ClosePositionReason) -> ClosedPosition {
        let asset_pnls = self.calculate_asset_pnls();

        ClosedPosition {
            pnl: Some(calculate_total_amount(
                &asset_pnls,
                &self.current_asset_prices,
            )),
            asset_pnls,
            open_date: self.open_date,
            open_asset_prices: self.open_asset_prices,
            activate_date: Some(self.activate_date),
            activate_price: Some(self.activate_price),
            activate_asset_prices: self.activate_asset_prices,
            close_date: DateTimeAsMicroseconds::now(),
            close_price: self.current_price,
            close_reason: reason,
            close_asset_prices: self.current_asset_prices.to_owned(),
            order: self.order,
            id: self.id,
        }
    }

    pub fn determine_close_reason(&self) -> Option<ClosePositionReason> {
        if self.is_stop_out() {
            return Some(ClosePositionReason::StopOut);
        }

        if self.is_stop_loss() {
            return Some(ClosePositionReason::StopLoss);
        }

        if self.is_take_profit() {
            return Some(ClosePositionReason::TakeProfit);
        }

        None
    }

    pub fn try_close(self) -> Position {
        let Some(reason) = self.determine_close_reason() else {
            return Position::Active(self);
        };

        Position::Closed(self.close(reason))
    }

    fn is_take_profit(&self) -> bool {
        if let Some(take_profit_config) = self.order.take_profit.as_ref() {
            let invest_amount = self
                .order
                .calculate_invest_amount(&self.current_asset_prices);
            let pnl = self.calculate_pnl(invest_amount);

            take_profit_config.is_triggered(pnl, self.current_price, &self.order.side)
        } else {
            false
        }
    }

    fn is_stop_loss(&self) -> bool {
        if let Some(stop_loss_config) = self.order.stop_loss.as_ref() {
            let invest_amount = self
                .order
                .calculate_invest_amount(&self.current_asset_prices);
            let pnl = self.calculate_pnl(invest_amount);

            stop_loss_config.is_triggered(pnl, self.current_price, &self.order.side)
        } else {
            false
        }
    }

    fn is_stop_out(&self) -> bool {
        let invest_amount = self
            .order
            .calculate_invest_amount(&self.current_asset_prices);
        let pnl = self.calculate_pnl(invest_amount);
        let margin_percent = calculate_margin_percent(invest_amount, pnl);

        100.0 - margin_percent >= self.order.stop_out_percent
    }

    pub fn is_margin_call(&self) -> bool {
        let invest_amount = self
            .order
            .calculate_invest_amount(&self.current_asset_prices);
        let pnl = self.calculate_pnl(invest_amount);
        let margin_percent = calculate_margin_percent(invest_amount, pnl);

        100.0 - margin_percent >= self.order.margin_call_percent
    }

    fn calculate_pnl(&self, invest_amount: f64) -> f64 {
        let volume = self.order.calculate_volume(invest_amount);

        match self.order.side {
            OrderSide::Buy => (self.current_price / self.activate_price - 1.0) * volume,
            OrderSide::Sell => (self.current_price / self.activate_price - 1.0) * -volume,
        }
    }

    pub fn calculate_asset_pnls(&self) -> HashMap<String, f64> {
        let mut pnls_by_assets = HashMap::with_capacity(self.order.invest_assets.len());

        for (asset, amount) in self.order.invest_assets.iter() {
            let pnl = self.calculate_pnl(*amount);
            let invest_amount = self.order.invest_assets.get(asset).expect("Impossible");
            let max_loss_amount = invest_amount * -1.0; // limit for isolated trade

            if pnl < max_loss_amount {
                pnls_by_assets.insert(asset.to_owned(), max_loss_amount);
            } else {
                pnls_by_assets.insert(asset.to_owned(), pnl);
            }
        }

        pnls_by_assets
    }
}

#[derive(Debug, Clone)]
pub struct ClosedPosition {
    pub id: String,
    pub order: Order,
    pub open_date: DateTimeAsMicroseconds,
    pub open_asset_prices: HashMap<String, f64>,
    pub activate_price: Option<f64>,
    pub activate_date: Option<DateTimeAsMicroseconds>,
    pub activate_asset_prices: HashMap<String, f64>,
    pub close_price: f64,
    pub close_date: DateTimeAsMicroseconds,
    pub close_reason: ClosePositionReason,
    pub close_asset_prices: HashMap<String, f64>,
    pub pnl: Option<f64>,
    pub asset_pnls: HashMap<String, f64>,
}

impl ClosedPosition {
    pub fn get_status(&self) -> PositionStatus {
        if self.activate_date.is_some() {
            PositionStatus::Filled
        } else {
            PositionStatus::Canceled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivePosition, ClosePositionReason};
    use crate::{
        orders::{Order, OrderSide, TakeProfitConfig},
        positions::{BidAsk, Position},
    };
    use rust_extensions::date_time::DateTimeAsMicroseconds;
    use std::collections::HashMap;

    #[tokio::test]
    async fn close_active_position() {
        let order = Order {
            base_asset: "USDT".to_string(),
            id: "test".to_string(),
            instrument: "ATOMUSDT".to_string(),
            trader_id: "test".to_string(),
            wallet_id: "test".to_string(),
            created_date: DateTimeAsMicroseconds::now(),
            desire_price: None,
            funding_fee_period: None,
            invest_assets: HashMap::from([("BTC".to_string(), 100.0)]),
            leverage: 1.0,
            side: OrderSide::Buy,
            take_profit: None,
            stop_loss: None,
            stop_out_percent: 10.0,
            margin_call_percent: 10.0,
            top_up_enabled: false,
            top_up_percent: 10.0,
        };
        let prices = HashMap::from([("BTC".to_string(), 22300.0)]);
        let bidask = BidAsk {
            ask: 14.748,
            bid: 14.748,
            datetime: DateTimeAsMicroseconds::now(),
            instrument: "ATOMUSDT".to_string(),
        };
        let position = order.open(&bidask, &prices);
        let mut position = match position {
            Position::Active(position) => position,
            _ => {
                panic!("Invalid position")
            }
        };

        position.current_price = 14.75;
        let closed_position = position.close(ClosePositionReason::ClientCommand);

        let pnl = closed_position.pnl.unwrap();
        let asset_pnl = *closed_position.asset_pnls.get("BTC").unwrap();

        assert_ne!(pnl, asset_pnl);
        assert_eq!(302.41388662883173, pnl);
        assert_eq!(0.01356116083537362, asset_pnl);
    }

    #[tokio::test]
    async fn close_by_tp() {
        let instrument = "ATOMUSDT".to_string();
        let prices = HashMap::from([("USDT".to_string(), 1.0)]);
        let invest_assets = HashMap::from([("USDT".to_string(), 100342.0)]);
        let order = new_order(instrument, invest_assets, 1.0, OrderSide::Sell);
        let bidask = BidAsk {
            ask: 13.815,
            bid: 13.815,
            datetime: DateTimeAsMicroseconds::now(),
            instrument: "ATOMUSDT".to_string(),
        };
        let mut position = new_active_position(order, &bidask, &prices);
        let take_profit = TakeProfitConfig {
            unit: crate::orders::AutoClosePositionUnit::PriceRate,
            value: 13.817,
        };
        position.set_take_profit(Some(take_profit));
        position.current_price = 13.817;

        let position = position.try_close();
        let _position = match position {
            Position::Closed(position) => position,
            _ => panic!("must be closed"),
        };
    }

    fn new_order(
        instrument: String,
        invest_assets: HashMap<String, f64>,
        leverage: f64,
        side: OrderSide,
    ) -> Order {
        Order {
            base_asset: "USDT".to_string(),
            id: "test".to_string(),
            instrument,
            trader_id: "test".to_string(),
            wallet_id: "test".to_string(),
            created_date: DateTimeAsMicroseconds::now(),
            desire_price: None,
            funding_fee_period: None,
            invest_assets,
            leverage,
            side,
            take_profit: None,
            stop_loss: None,
            stop_out_percent: 90.0,
            margin_call_percent: 70.0,
            top_up_enabled: false,
            top_up_percent: 10.0,
        }
    }

    fn new_active_position(
        order: Order,
        bidask: &BidAsk,
        asset_prices: &HashMap<String, f64>,
    ) -> ActivePosition {
        let now = DateTimeAsMicroseconds::now();

        ActivePosition {
            id: Position::generate_id(),
            open_date: now,
            open_asset_prices: asset_prices.to_owned(),
            activate_price: bidask.get_open_price(&order.side),
            activate_date: now,
            activate_asset_prices: asset_prices.to_owned(),
            current_price: bidask.get_close_price(&order.side),
            current_asset_prices: asset_prices.to_owned(),
            last_update_date: now,
            order,
        }
    }
}
