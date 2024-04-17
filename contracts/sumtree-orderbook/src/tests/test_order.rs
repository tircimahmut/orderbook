use crate::{
    constants::{MAX_TICK, MIN_TICK},
    error::ContractError,
    order::*,
    orderbook::*,
    state::*,
    sumtree::{
        node::{NodeType, TreeNode},
        tree::get_root_node,
    },
    tests::test_utils::{decimal256_from_u128, place_multiple_limit_orders},
    types::{
        FilterOwnerOrders, LimitOrder, MarketOrder, OrderDirection, TickValues, REPLY_ID_CLAIM,
        REPLY_ID_CLAIM_BOUNTY, REPLY_ID_REFUND,
    },
};
use cosmwasm_std::{
    coin, testing::mock_dependencies, Addr, BankMsg, Coin, Decimal, Empty, SubMsg, Uint128,
};
use cosmwasm_std::{
    testing::{mock_dependencies_with_balances, mock_env, mock_info},
    Decimal256,
};
use cw_utils::PaymentError;

use super::test_utils::{
    format_test_name, generate_limit_orders, OrderOperation, LARGE_NEGATIVE_TICK,
    LARGE_POSITIVE_TICK,
};

struct PlaceLimitTestCase {
    name: &'static str,
    tick_id: i64,
    quantity: Uint128,
    sent: Uint128,
    order_direction: OrderDirection,
    claim_bounty: Option<Decimal>,
    expected_error: Option<ContractError>,
}

#[test]
fn test_place_limit() {
    let test_cases = vec![
        PlaceLimitTestCase {
            name: "valid order with positive tick id",
            tick_id: 10,
            quantity: Uint128::new(100),
            sent: Uint128::new(100),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: None,
        },
        PlaceLimitTestCase {
            name: "valid order with zero tick id",
            tick_id: 0,
            quantity: Uint128::new(34321),
            sent: Uint128::new(34321),
            order_direction: OrderDirection::Bid,
            claim_bounty: None,
            expected_error: None,
        },
        PlaceLimitTestCase {
            name: "valid order with negative tick id",
            tick_id: -5,
            quantity: Uint128::new(100),
            sent: Uint128::new(100),
            order_direction: OrderDirection::Bid,
            claim_bounty: None,
            expected_error: None,
        },
        PlaceLimitTestCase {
            name: "valid order with large quantity",
            tick_id: 3,
            quantity: Uint128::new(34321),
            sent: Uint128::new(34321),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: None,
        },
        PlaceLimitTestCase {
            name: "valid order with claim bounty",
            tick_id: 10,
            quantity: Uint128::new(100),
            sent: Uint128::new(100),
            order_direction: OrderDirection::Ask,
            claim_bounty: Some(Decimal::percent(10)),
            expected_error: None,
        },
        PlaceLimitTestCase {
            name: "order with claim bounty > 1 (invalid)",
            tick_id: 10,
            quantity: Uint128::new(100),
            sent: Uint128::new(100),
            order_direction: OrderDirection::Ask,
            claim_bounty: Some(Decimal::one() + Decimal::one()),
            expected_error: Some(ContractError::InvalidClaimBounty {
                claim_bounty: Some(Decimal::one() + Decimal::one()),
            }),
        },
        PlaceLimitTestCase {
            name: "invalid tick id (max)",
            tick_id: MAX_TICK + 1,
            quantity: Uint128::new(100),
            sent: Uint128::new(100),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: Some(ContractError::InvalidTickId {
                tick_id: MAX_TICK + 1,
            }),
        },
        PlaceLimitTestCase {
            name: "invalid tick id (min)",
            tick_id: MIN_TICK - 1,
            quantity: Uint128::new(100),
            sent: Uint128::new(100),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: Some(ContractError::InvalidTickId {
                tick_id: MIN_TICK - 1,
            }),
        },
        PlaceLimitTestCase {
            name: "invalid quantity",
            tick_id: 1,
            quantity: Uint128::zero(),
            sent: Uint128::new(1000),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: Some(ContractError::InvalidQuantity {
                quantity: Uint128::zero(),
            }),
        },
        PlaceLimitTestCase {
            name: "insufficient funds",
            tick_id: 1,
            quantity: Uint128::new(1000),
            sent: Uint128::new(500),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: Some(ContractError::InsufficientFunds {
                sent: Uint128::new(500),
                required: Uint128::new(1000),
            }),
        },
        PlaceLimitTestCase {
            name: "excessive funds",
            tick_id: 1,
            quantity: Uint128::new(100),
            sent: Uint128::new(500),
            order_direction: OrderDirection::Ask,
            claim_bounty: None,
            expected_error: Some(ContractError::InsufficientFunds {
                sent: Uint128::new(500),
                required: Uint128::new(100),
            }),
        },
    ];

    for test in test_cases {
        // --- Setup ---

        // Create a mock environment and info
        let coin_vec = vec![coin(
            test.sent.u128(),
            if test.order_direction == OrderDirection::Ask {
                "base"
            } else {
                "quote"
            },
        )];
        let balances = [("creator", coin_vec.as_slice())];
        let mut deps = mock_dependencies_with_balances(&balances);
        let env = mock_env();
        let info = mock_info("creator", &coin_vec);

        // Create an orderbook to operate on
        let quote_denom = "quote".to_string();
        let base_denom = "base".to_string();
        create_orderbook(deps.as_mut(), quote_denom, base_denom).unwrap();

        // --- System under test ---

        let response = place_limit(
            &mut deps.as_mut(),
            env.clone(),
            info.clone(),
            test.tick_id,
            test.order_direction,
            test.quantity,
            test.claim_bounty,
        );

        // --- Assertions ---

        // Error case assertions if applicable
        if let Some(expected_error) = &test.expected_error {
            assert_eq!(
                response.unwrap_err(),
                *expected_error,
                "{}",
                format_test_name(test.name)
            );

            // Verify that the order was not put in state
            let order_result = orders()
                .may_load(&deps.storage, &(test.tick_id, 0))
                .unwrap();
            assert!(order_result.is_none(), "{}", format_test_name(test.name));

            // Verify liquidity was not updated
            let state = TICK_STATE
                .load(&deps.storage, test.tick_id)
                .unwrap_or_default();
            let values = state.get_values(test.order_direction);
            assert!(
                values.total_amount_of_liquidity.is_zero(),
                "{}",
                format_test_name(test.name)
            );
            continue;
        }

        // Assert no error and retrieve response contents
        let response = response.unwrap();

        // Assertions on the response for a valid order
        assert_eq!(
            response.attributes[0],
            ("method", "placeLimit"),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[1],
            ("owner", "creator"),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[2],
            ("tick_id", test.tick_id.to_string()),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[5],
            ("quantity", test.quantity.to_string()),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[6],
            ("quantity_fulfilled", "0"),
            "{}",
            format_test_name(test.name)
        );

        // Retrieve the order from storage to verify it was saved correctly
        let expected_order_id = 0;
        let order = orders()
            .load(&deps.storage, &(test.tick_id, expected_order_id))
            .unwrap();

        assert_eq!(
            order.tick_id,
            test.tick_id,
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            order.order_id,
            expected_order_id,
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            order.order_direction,
            test.order_direction,
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            order.owner,
            Addr::unchecked("creator"),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            order.quantity,
            test.quantity,
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(order.etas, Decimal256::zero());

        // Validate liquidity updated as intended
        let state = TICK_STATE
            .load(&deps.storage, test.tick_id)
            .unwrap()
            .get_values(test.order_direction);
        assert_eq!(
            state.total_amount_of_liquidity,
            decimal256_from_u128(test.quantity),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            state.cumulative_total_value,
            decimal256_from_u128(test.quantity),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            state.effective_total_amount_swapped,
            Decimal256::zero(),
            "{}",
            format_test_name(test.name)
        );
    }
}

struct CancelLimitTestCase {
    name: &'static str,

    tick_id: i64,
    order_id: u64,
    order_direction: OrderDirection,
    quantity: Uint128,
    place_order: bool,
    expected_error: Option<ContractError>,
    owner: &'static str,
    sender: Option<&'static str>,
    sent: Vec<Coin>,
}

#[test]
fn test_cancel_limit() {
    let direction = OrderDirection::Ask;
    let test_cases = vec![
        CancelLimitTestCase {
            name: "valid order cancel",

            tick_id: 1,
            order_id: 0,
            order_direction: OrderDirection::Ask,
            quantity: Uint128::from(100u128),
            place_order: true,
            expected_error: None,
            owner: "creator",
            sender: None,
            sent: vec![],
        },
        CancelLimitTestCase {
            name: "sent funds accidentally",

            tick_id: 1,
            order_id: 0,
            order_direction: OrderDirection::Ask,
            quantity: Uint128::from(100u128),
            place_order: true,
            expected_error: Some(ContractError::PaymentError(PaymentError::NonPayable {})),
            owner: "creator",
            sender: None,
            sent: vec![coin(100, "quote")],
        },
        CancelLimitTestCase {
            name: "unauthorized cancel (not owner)",

            tick_id: 1,
            order_id: 0,
            order_direction: OrderDirection::Ask,
            quantity: Uint128::from(100u128),
            place_order: true,
            expected_error: Some(ContractError::Unauthorized {}),
            owner: "creator",
            sender: Some("malicious_user"),
            sent: vec![],
        },
        CancelLimitTestCase {
            name: "order not found",

            tick_id: 1,
            order_id: 0,
            order_direction: OrderDirection::Ask,
            quantity: Uint128::from(100u128),
            place_order: false,
            expected_error: Some(ContractError::OrderNotFound {
                tick_id: 1,
                order_id: 0,
            }),
            owner: "creator",
            sender: None,
            sent: vec![],
        },
    ];

    for test in test_cases {
        // --- Setup ---

        // Create a mock environment and info
        let balances = [(test.owner, test.sent.as_slice())];
        let mut deps = mock_dependencies_with_balances(&balances);
        let env = mock_env();
        let info = mock_info(test.sender.unwrap_or(test.owner), test.sent.as_slice());

        // Create an orderbook to operate on
        let quote_denom = "quote".to_string();
        let base_denom = "base".to_string();
        create_orderbook(deps.as_mut(), quote_denom.clone(), base_denom.clone()).unwrap();

        if test.place_order {
            let place_info = mock_info(
                test.owner,
                &[coin(test.quantity.u128(), base_denom.clone())],
            );
            place_limit(
                &mut deps.as_mut(),
                env.clone(),
                place_info,
                test.tick_id,
                test.order_direction,
                test.quantity,
                None,
            )
            .unwrap();
        }

        // --- System under test ---

        let response = cancel_limit(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            test.tick_id,
            test.order_id,
        );

        // --- Assertions ---

        // Error case assertions if applicable
        if let Some(expected_error) = &test.expected_error {
            assert_eq!(
                response.unwrap_err(),
                *expected_error,
                "{}",
                format_test_name(test.name)
            );

            // Verify that the order was not put in state
            let order_result = orders()
                .may_load(&deps.storage, &(test.tick_id, test.order_id))
                .unwrap();
            assert!(
                order_result.is_some() == test.place_order,
                "{}",
                format_test_name(test.name)
            );

            // Verify Liqudity was updated as intended
            let state = TICK_STATE
                .load(deps.as_ref().storage, test.tick_id)
                .unwrap_or_default()
                .get_values(test.order_direction);
            if test.place_order {
                assert_eq!(
                    state.total_amount_of_liquidity,
                    decimal256_from_u128(test.quantity),
                    "{}",
                    format_test_name(test.name)
                );
            } else {
                assert!(
                    state.total_amount_of_liquidity.is_zero(),
                    "{}",
                    format_test_name(test.name)
                );
            }
            continue;
        }

        // Assert no error and retrieve response contents
        let response = response.unwrap();
        let refund_denom = match test.order_direction {
            OrderDirection::Bid => quote_denom.clone(),
            OrderDirection::Ask => base_denom.clone(),
        };
        let expected_refund_msg: SubMsg<Empty> = SubMsg::reply_on_error(
            BankMsg::Send {
                to_address: test.owner.to_string(),
                amount: vec![coin(test.quantity.u128(), refund_denom)],
            },
            REPLY_ID_REFUND,
        );

        // Assertions on the response for a valid order
        assert_eq!(
            response.attributes[0],
            ("method", "cancelLimit"),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[1],
            ("owner", test.owner),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[2],
            ("tick_id", test.tick_id.to_string()),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.attributes[3],
            ("order_id", test.order_id.to_string()),
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.messages.len(),
            1,
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(
            response.messages[0],
            expected_refund_msg,
            "{}",
            format_test_name(test.name)
        );

        // Retrieve the order from storage to verify it was saved correctly
        let expected_order_id = 0;
        let order = orders()
            .may_load(&deps.storage, &(test.tick_id, expected_order_id))
            .unwrap();

        // Verify the order's fields
        assert!(order.is_none(), "{}", format_test_name(test.name));

        // Validate liquidity updated as intended
        let state = TICK_STATE
            .load(deps.as_ref().storage, test.tick_id)
            .unwrap_or_default()
            .get_values(test.order_direction);

        assert!(
            state.total_amount_of_liquidity.is_zero(),
            "{}",
            format_test_name(test.name)
        );

        // -- Sumtree --

        // Ensure tree is saved correctly
        let tree = get_root_node(deps.as_ref().storage, test.tick_id, direction).unwrap();

        // Traverse the tree to check its form
        let res = tree.traverse(deps.as_ref().storage).unwrap();
        let mut root_node = TreeNode::new(
            test.tick_id,
            direction,
            1,
            NodeType::internal_uint256(test.quantity, (0u128, test.quantity)),
        );
        root_node.set_weight(2).unwrap();
        let mut cancelled_node = TreeNode::new(
            test.tick_id,
            direction,
            2,
            NodeType::leaf_uint256(0u128, test.quantity),
        );
        root_node.left = Some(cancelled_node.key);
        cancelled_node.parent = Some(root_node.key);

        // Ensure tree traversal returns expected ordering
        assert_eq!(res, vec![root_node, cancelled_node])
    }
}

struct RunMarketOrderTestCase {
    name: &'static str,
    placed_order: MarketOrder,
    tick_bound: i64,
    orders: Vec<LimitOrder>,
    sent: Uint128,
    expected_output: Uint128,
    expected_tick_etas: Vec<(i64, Decimal256)>,
    expected_tick_pointers: Vec<(OrderDirection, i64)>,
    expected_error: Option<ContractError>,
}

#[test]
fn test_run_market_order() {
    let quote_denom = "quote";
    let base_denom = "base";
    // TODO: move these defaults to global scope or helper file
    let default_current_tick = 0;
    let default_owner = "creator";
    let default_sender = "sender";
    let default_quantity = Uint128::new(100);
    let test_cases = vec![
        RunMarketOrderTestCase {
            name: "happy path bid at negative tick",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MAX_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[-1500000],
                // Current tick is below the active limit orders
                -2500000,
                // 1000 units of liquidity total
                10,
                default_quantity,
            ),

            // Bidding 1000 units of input into tick -1500000, which corresponds to $0.85,
            // implies 1000*0.85 = 850 units of output.
            expected_output: Uint128::new(850),
            expected_tick_etas: vec![(-1500000, decimal256_from_u128(Uint128::new(850)))],
            expected_tick_pointers: vec![(OrderDirection::Ask, -1500000)],
            expected_error: None,
        },
        RunMarketOrderTestCase {
            name: "happy path bid at positive tick",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MAX_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[40000000],
                // Current tick is below the active limit orders
                default_current_tick,
                // Two orders with sufficient total liquidity to process the
                // full market order
                2,
                Uint128::new(25_000_000),
            ),

            // Bidding 1000 units of input into tick 40,000,000, which corresponds to a
            // price of $50000 (from tick math test cases).
            //
            // This implies 1000*50000 = 50,000,000 units of output.
            expected_output: Uint128::new(50_000_000),
            expected_tick_etas: vec![(40000000, decimal256_from_u128(Uint128::new(50_000_000)))],
            expected_tick_pointers: vec![(OrderDirection::Ask, 40000000)],
            expected_error: None,
        },
        RunMarketOrderTestCase {
            name: "bid at very small negative tick",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MAX_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[-17765433],
                // Current tick is below the active limit orders
                -20000000,
                // Four limit orders with sufficient total liquidity to process the
                // full market order
                4,
                Uint128::new(3),
            ),

            // Bidding 1000 units of input into tick -17765433, which corresponds to a
            // price of $0.012345670000000000 (from tick math test cases).
            //
            // This implies 1000*0.012345670000000000 = 12.34567 units of output,
            // truncated to 12 units.
            expected_output: Uint128::new(12),
            expected_tick_etas: vec![(-17765433, decimal256_from_u128(Uint128::new(12)))],
            expected_tick_pointers: vec![(OrderDirection::Ask, -17765433)],
            expected_error: None,
        },
        RunMarketOrderTestCase {
            name: "bid across multiple ticks",
            sent: Uint128::new(589 + 1),
            placed_order: MarketOrder::new(
                Uint128::new(589 + 1),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MAX_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[-1500000, 40000000],
                // Current tick is below the active limit orders
                -2500000,
                // 500 units of liquidity on each tick
                5,
                default_quantity,
            ),

            // Bidding 1000 units of input into tick -1500000, which corresponds to $0.85,
            // implies 1000*0.85 = 850 units of output, but there is only 500 on the tick.
            //
            // So 500 gets filled at -1500000, corresponding to ~589 of the input (500/0.85).
            // The remaining 1 unit is filled at tick 40,000,000 (price $50,000), which
            // corresponds to the remaining liquidity.
            //
            // Thus, the total expected output is 502.
            //
            // Note: this case does not cover rounding for input consumption since it overfills
            // the tick.
            expected_output: Uint128::new(1000),
            expected_tick_etas: vec![
                (-1500000, decimal256_from_u128(Uint128::new(500))),
                (40000000, decimal256_from_u128(Uint128::new(500))),
            ],
            expected_tick_pointers: vec![(OrderDirection::Ask, 40000000)],
            expected_error: None,
        },
        RunMarketOrderTestCase {
            name: "happy path ask at positive tick",
            sent: Uint128::new(100000),
            placed_order: MarketOrder::new(
                Uint128::new(100000),
                OrderDirection::Ask,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MIN_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[40000000],
                // Current tick is above the active limit orders
                40000000 + 1,
                // Two orders with sufficient total liquidity to process the
                // full market order
                2,
                Uint128::new(1),
            ),

            // Asking 100,000 units of input into tick 40,000,000, which corresponds to a
            // price of $1/50000 (from tick math test cases).
            //
            // This implies 100,000/50000 = 2 units of output.
            expected_output: Uint128::new(2),
            expected_tick_etas: vec![(40000000, decimal256_from_u128(Uint128::new(2)))],
            expected_tick_pointers: vec![(OrderDirection::Bid, 40000000)],
            expected_error: None,
        },
        RunMarketOrderTestCase {
            name: "ask at negative tick",
            sent: Uint128::new(100000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Ask,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MIN_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[-17765433],
                // Current tick is above the active limit orders
                default_current_tick,
                // Two orders with sufficient total liquidity to process the
                // full market order
                2,
                Uint128::new(50_000),
            ),

            // The order asks with 1000 units of input into tick -17765433, which corresponds
            // to a price of $0.012345670000000000 (from tick math test cases).
            //
            // This implies 1000 / 0.012345670000000000 = 81,000.059 units of output,
            // which gets truncated to 81,000 units.
            expected_output: Uint128::new(81_000),
            expected_tick_etas: vec![(-17765433, decimal256_from_u128(Uint128::new(81_000)))],
            expected_tick_pointers: vec![(OrderDirection::Bid, -17765433)],
            expected_error: None,
        },
        RunMarketOrderTestCase {
            name: "invalid tick bound for bid",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MIN_TICK - 1,
            // Orders we expect to not get touched
            orders: generate_limit_orders(&[10], default_current_tick, 10, Uint128::new(10)),
            expected_output: Uint128::zero(),
            expected_tick_etas: vec![(10, Decimal256::zero())],
            expected_tick_pointers: vec![(OrderDirection::Ask, 10)],
            expected_error: Some(ContractError::InvalidTickId {
                tick_id: MIN_TICK - 1,
            }),
        },
        RunMarketOrderTestCase {
            name: "invalid tick bound for ask",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Ask,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MAX_TICK + 1,
            // Orders we expect to not get touched
            orders: generate_limit_orders(&[10], default_current_tick, 10, Uint128::new(10)),
            expected_output: Uint128::zero(),
            expected_tick_etas: vec![(10, Decimal256::zero())],
            expected_tick_pointers: vec![(OrderDirection::Bid, MIN_TICK)],
            expected_error: Some(ContractError::InvalidTickId {
                tick_id: MAX_TICK + 1,
            }),
        },
        RunMarketOrderTestCase {
            name: "invalid tick bound due to bid direction",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            // We expect the target tick for a market bid to be above the current tick,
            // but this is below.
            tick_bound: MIN_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[-1500000],
                // Current tick is below the active limit orders
                -2500000,
                // 1000 units of liquidity total
                10,
                default_quantity,
            ),

            expected_output: Uint128::zero(),
            expected_tick_etas: vec![(-1500000, Decimal256::zero())],
            expected_tick_pointers: vec![(OrderDirection::Ask, -1500000)],
            expected_error: Some(ContractError::InvalidTickId { tick_id: MIN_TICK }),
        },
        RunMarketOrderTestCase {
            name: "bid at positive tick that can only partially be filled",
            sent: Uint128::new(1000),
            placed_order: MarketOrder::new(
                Uint128::new(1000),
                OrderDirection::Bid,
                Addr::unchecked(default_sender),
            ),
            tick_bound: MAX_TICK,

            // Orders to fill against
            orders: generate_limit_orders(
                &[40000000],
                // Current tick is below the active limit orders
                default_current_tick,
                // Only half the required liquidity to process the full input.
                1,
                Uint128::new(25_000_000),
            ),

            // Bidding 1000 units of input into tick 40,000,000, which corresponds to a
            // price of $50000 (from tick math test cases).
            //
            // This implies 1000*50000 = 50,000,000 units of output.
            //
            // However, since the book only has 25,000,000 units of liquidity, that is how much
            // is filled.
            expected_output: Uint128::new(25_000_000),
            expected_tick_etas: vec![(40000000, decimal256_from_u128(Uint128::new(25_000_000)))],
            expected_tick_pointers: vec![(OrderDirection::Ask, 40000000)],
            expected_error: None,
        },
    ];

    for test in test_cases {
        // --- Setup ---

        // Create a mock environment and info
        let coin_vec = vec![coin(
            test.sent.u128(),
            if test.placed_order.order_direction == OrderDirection::Ask {
                base_denom
            } else {
                quote_denom
            },
        )];
        let balances = [(default_sender, coin_vec.as_slice())];
        let mut deps = mock_dependencies_with_balances(&balances);
        let env = mock_env();

        // Create an orderbook to operate on
        create_orderbook(
            deps.as_mut(),
            quote_denom.to_string(),
            base_denom.to_string(),
        )
        .unwrap();

        // Place limit orders on orderbook
        place_multiple_limit_orders(&mut deps.as_mut(), env.clone(), default_owner, test.orders)
            .unwrap();

        // We store order state before to run assertions later
        let orders_before = get_orders_by_owner(
            &deps.storage,
            FilterOwnerOrders::all(Addr::unchecked(default_owner)),
            None,
            None,
            None,
        )
        .unwrap();

        // --- System under test ---

        let mut market_order = test.placed_order.clone();
        let response = run_market_order(deps.as_mut().storage, &mut market_order, test.tick_bound);

        // --- Assertions ---

        // Assert expected tick ETAS values are correct.
        // This should run regardless of whether we error or not.
        for (tick_id, expected_etas) in test.expected_tick_etas {
            let tick_state = TICK_STATE
                .load(&deps.storage, tick_id)
                .unwrap()
                .get_values(test.placed_order.order_direction.opposite());
            assert_eq!(
                expected_etas,
                tick_state.effective_total_amount_swapped,
                "{}",
                format_test_name(test.name)
            );
        }

        // Assert orderbook tick pointers were updated as expected
        let post_process_orderbook = ORDERBOOK.load(deps.as_ref().storage).unwrap();
        for (direction, tick_id) in test.expected_tick_pointers {
            let pointer = match direction {
                OrderDirection::Ask => post_process_orderbook.next_ask_tick,
                OrderDirection::Bid => post_process_orderbook.next_bid_tick,
            };
            assert_eq!(tick_id, pointer, "{}", format_test_name(test.name));
        }

        // Regardless of whether we error, orders should not be modified.
        let orders_after = get_orders_by_owner(
            &deps.storage,
            FilterOwnerOrders::all(Addr::unchecked(default_owner)),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            orders_before,
            orders_after,
            "{}",
            format_test_name(test.name)
        );

        // Error case assertions if applicable
        if let Some(expected_error) = &test.expected_error {
            assert_eq!(
                *expected_error,
                response.unwrap_err(),
                "{}",
                format_test_name(test.name)
            );

            continue;
        }

        // Assert no error
        let response = response.unwrap();

        // We expect the output denom to be the opposite of the input denom,
        // although we derive it directly from the order direction to ensure correctness.
        let expected_denom = match test.placed_order.order_direction {
            OrderDirection::Bid => base_denom,
            OrderDirection::Ask => quote_denom,
        };
        let expected_msg = BankMsg::Send {
            to_address: default_sender.to_string(),
            amount: vec![coin(test.expected_output.u128(), expected_denom)],
        };

        // Ensure output is as expected
        assert_eq!(
            test.expected_output,
            response.0,
            "{}",
            format_test_name(test.name)
        );
        assert_eq!(expected_msg, response.1, "{}", format_test_name(test.name));
    }
}

struct RunMarketOrderMovingTickTestCase {
    name: &'static str,
    operations: Vec<OrderOperation>,
    // (tick_id, direction), (etas, ctt)
    expected_tick_values: Vec<((i64, OrderDirection), TickValues)>,
}

#[test]
fn test_run_market_order_moving_tick() {
    let env = mock_env();
    let info = mock_info("sender", &[]);
    let test_cases: Vec<RunMarketOrderMovingTickTestCase> = vec![
        RunMarketOrderMovingTickTestCase {
            name: "positive tick movement on filled market bid",
            operations: vec![
                // Place Ask on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Place Ask on second tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    1,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill all limits on tick 0 and 50% of tick 1, leaving tick 0 empty and forcing positive movement
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(15u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                // Place Bid on first tick to create overlapping state
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
            ],
            expected_tick_values: vec![
                (
                    (0, OrderDirection::Ask),
                    TickValues {
                        // Entire tick has been filled
                        effective_total_amount_swapped: decimal256_from_u128(10u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: Decimal256::zero(),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (1, OrderDirection::Ask),
                    TickValues {
                        // 50% of this tick has been filled
                        effective_total_amount_swapped: decimal256_from_u128(5u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: decimal256_from_u128(5u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (0, OrderDirection::Bid),
                    TickValues {
                        // None of this tick has been filled
                        effective_total_amount_swapped: Decimal256::zero(),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: decimal256_from_u128(10u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
            ],
        },
        RunMarketOrderMovingTickTestCase {
            name: "negative tick movement on filled market ask",
            operations: vec![
                // Place Bid on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Place Bid on negative tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    -1,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill entire first tick and 50% of next tick to force negative movement
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(15u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
                // Place Ask on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
            ],
            expected_tick_values: vec![
                (
                    (0, OrderDirection::Bid),
                    TickValues {
                        // Entire tick has been filled
                        effective_total_amount_swapped: decimal256_from_u128(10u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: Decimal256::zero(),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (-1, OrderDirection::Bid),
                    TickValues {
                        // 50% of tick has been filled
                        effective_total_amount_swapped: decimal256_from_u128(5u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: decimal256_from_u128(5u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (0, OrderDirection::Ask),
                    TickValues {
                        // None of tick has been filled (overlapping state)
                        effective_total_amount_swapped: Decimal256::zero(),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: decimal256_from_u128(10u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
            ],
        },
        RunMarketOrderMovingTickTestCase {
            name: "negative tick movement followed by positive movement",
            operations: vec![
                // Place Bid on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Place Bid on negative tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    -1,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill entire first tick and 50% of next tick to force negative movement
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(15u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
                // Place Ask on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill entire ask to force positive movement
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                // Place Bid on first tick to update previous state
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(12u128),
                    Decimal256::zero(),
                    None,
                )),
            ],
            expected_tick_values: vec![
                (
                    // Recall that each tick has two sets of values (one for each order direction).
                    // (0, OrderDirection::Bid) corresponds to the bid values of tick 0.
                    (0, OrderDirection::Bid),
                    TickValues {
                        // Tick was originally filled on negative movement
                        // A total value of 12 remains at the end of these swaps
                        // 10 filled from first movement, 12 placed after second
                        effective_total_amount_swapped: decimal256_from_u128(10u128),
                        cumulative_total_value: decimal256_from_u128(22u128),
                        total_amount_of_liquidity: decimal256_from_u128(12u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (-1, OrderDirection::Bid),
                    TickValues {
                        // 50% of tick filled
                        effective_total_amount_swapped: decimal256_from_u128(5u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: decimal256_from_u128(5u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (0, OrderDirection::Ask),
                    TickValues {
                        // Entire tick filled
                        effective_total_amount_swapped: decimal256_from_u128(10u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: Decimal256::zero(),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
            ],
        },
        RunMarketOrderMovingTickTestCase {
            name: "positive tick movement followed by negative movement",
            operations: vec![
                // Place Ask on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Place Ask on second tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    1,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill entire first tick and 50% of second tick to force positive movement
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(15u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                // Place Bid on first tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Bid,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill entire first tick to force negative movement
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
                // Place Ask on first tick to update previous state
                OrderOperation::PlaceLimit(LimitOrder::new(
                    0,
                    0,
                    OrderDirection::Ask,
                    Addr::unchecked(info.sender.as_str()),
                    Uint128::from(12u128),
                    Decimal256::zero(),
                    None,
                )),
            ],
            expected_tick_values: vec![
                (
                    (0, OrderDirection::Ask),
                    TickValues {
                        // Tick was originally filled on positive movement
                        // A total value of 12 remains at the end of these swaps
                        // 10 filled from first movement, 12 placed after second
                        effective_total_amount_swapped: decimal256_from_u128(10u128),
                        cumulative_total_value: decimal256_from_u128(22u128),
                        total_amount_of_liquidity: decimal256_from_u128(12u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (1, OrderDirection::Ask),
                    TickValues {
                        // Tick 50% filled
                        effective_total_amount_swapped: decimal256_from_u128(5u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: decimal256_from_u128(5u128),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
                (
                    (0, OrderDirection::Bid),
                    TickValues {
                        // Tick entirely filled
                        effective_total_amount_swapped: decimal256_from_u128(10u128),
                        cumulative_total_value: decimal256_from_u128(10u128),
                        total_amount_of_liquidity: Decimal256::zero(),
                        cumulative_realized_cancels: Decimal256::zero(),
                        last_tick_sync_etas: Decimal256::zero(),
                    },
                ),
            ],
        },
    ];

    for test in test_cases {
        let mut deps = mock_dependencies();

        let quote_denom = "quote";
        let base_denom = "base";
        create_orderbook(
            deps.as_mut(),
            quote_denom.to_string(),
            base_denom.to_string(),
        )
        .unwrap();

        for operation in test.operations {
            operation
                .run(deps.as_mut(), env.clone(), info.clone())
                .unwrap();
        }

        for ((tick_id, direction), values) in test.expected_tick_values {
            let tick_state = TICK_STATE.load(deps.as_ref().storage, tick_id).unwrap();
            let tick_values = tick_state.get_values(direction);

            assert_eq!(tick_values, values, "{}", format_test_name(test.name))
        }
    }
}

struct ClaimOrderTestCase {
    name: &'static str,
    operations: Vec<OrderOperation>,
    sender: Addr,

    tick_id: i64,
    order_id: u64,

    expected_bank_msg: SubMsg,
    expected_bounty_msg: Option<SubMsg>,

    expected_order_state: Option<LimitOrder>,
    expected_error: Option<ContractError>,
}

#[test]
fn test_claim_order() {
    let valid_tick_id = 0;
    let quote_denom = "quote";
    let base_denom = "base";
    let sender = Addr::unchecked("sender");
    let test_cases: Vec<ClaimOrderTestCase> = vec![
        // A tick id of 0 operates on a tick price of 1
        ClaimOrderTestCase {
            name: "ASK: valid basic full claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(10u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid basic full claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(10u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid basic partial claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(5u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                0,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(5u128),
                decimal256_from_u128(5u128),
                None,
            )),
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid two-step partial claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(7u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                OrderOperation::Claim((valid_tick_id, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(3u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(3u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid basic full claim with claim bounty",
            sender: Addr::unchecked("claimer"),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    Some(Decimal::percent(10)),
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    // Ensure the order placer receives the claimed amount
                    to_address: sender.to_string(),
                    // 10% of the claimed amount goes to the bounty
                    amount: vec![coin(10u128 - 1u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: Some(SubMsg::reply_on_error(
                BankMsg::Send {
                    // Ensure the claimer receives the bounty
                    to_address: Addr::unchecked("claimer").to_string(),
                    // 10% of the claimed amount goes to the bounty
                    amount: vec![coin(1u128, quote_denom)],
                },
                REPLY_ID_CLAIM_BOUNTY,
            )),
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid two-step partial claim with claim bounty",
            sender: Addr::unchecked("claimer"),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    // 35% claim bounty (0.35)
                    Some(Decimal::percent(35)),
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(7u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                OrderOperation::Claim((valid_tick_id, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(3u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // 35% of most recent claim goes to bounty: 3*0.35 = 1.05 -> 1 unit
                    amount: vec![coin(3u128 - 1u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: Some(SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: Addr::unchecked("claimer").to_string(),
                    // 1 unit goes to claimer for bounty
                    amount: vec![coin(1u128, quote_denom)],
                },
                REPLY_ID_CLAIM_BOUNTY,
            )),
            expected_order_state: None,
            expected_error: None,
        },
        // All large positive tick orders operate on a tick price of 2
        ClaimOrderTestCase {
            name: "ASK: valid basic full claim (large positive tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_POSITIVE_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    // Tick price is 2, 2*5 = 10
                    Uint128::from(5u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_POSITIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 2, 10/2 = 5
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid basic partial claim (large positive tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_POSITIVE_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    // Tick price is 2, 2*2 = 4
                    Uint128::from(2u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_POSITIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 2, 4/2 = 2
                    amount: vec![coin(2u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                LARGE_POSITIVE_TICK,
                0,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(6u128),
                decimal256_from_u128(4u128),
                None,
            )),
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid two-step partial claim (large positive tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_POSITIVE_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(2u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                // Claim the first partial fill
                OrderOperation::Claim((LARGE_POSITIVE_TICK, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    // Tick price is 2, 2*3 = 6
                    Uint128::from(3u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_POSITIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 2, 6/2 = 3
                    amount: vec![coin(3u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        // All Large Negative Tick orders operate on a tick price of 0.5
        ClaimOrderTestCase {
            name: "ASK: valid basic full claim (large negative tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_NEGATIVE_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(200u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_NEGATIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(200u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid basic partial claim (large negative tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_NEGATIVE_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_NEGATIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(100u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                LARGE_NEGATIVE_TICK,
                0,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(50u128),
                decimal256_from_u128(50u128),
                None,
            )),
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid two-step partial claim (large negative tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_NEGATIVE_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
                // Claim the first partial fill
                OrderOperation::Claim((LARGE_NEGATIVE_TICK, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_NEGATIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(100u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: full claim with a previous cancellation",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::Cancel((valid_tick_id, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(100u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "ASK: valid basic full claim at MIN_TICK",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    MIN_TICK,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    // Tick price is 0.000000000001, so 3_333_333_333_333 * 0.000000000001 = 3.33333333333
                    // We expect this to get truncated to 3, as order outputs should always be rounding
                    // in favor of the orderbook.
                    Uint128::from(3_333_333_333_333u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: MIN_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 0.000000000001, 3 / 0.000000000001 = 3_000_000_000_000
                    amount: vec![coin(3_000_000_000_000u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                MIN_TICK,
                0,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(7u128),
                decimal256_from_u128(3u128),
                None,
            )),
            expected_error: None,
        },
        // A tick id of 0 operates on a tick price of 1
        ClaimOrderTestCase {
            name: "BID: valid basic full claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(10u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: valid basic partial claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(5u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                0,
                OrderDirection::Bid,
                sender.clone(),
                Uint128::from(5u128),
                decimal256_from_u128(5u128),
                None,
            )),
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: valid two-step partial claim",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(7u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
                OrderOperation::Claim((valid_tick_id, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(3u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(3u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        // All large positive tick orders operate on a tick price of 2
        ClaimOrderTestCase {
            name: "BID: valid basic full claim (large positive tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_POSITIVE_TICK,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    // Tick price is 2, 2*5 = 10
                    Uint128::from(20u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_POSITIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 2, 10/2 = 5
                    amount: vec![coin(20u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: valid basic partial claim (large positive tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_POSITIVE_TICK,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_POSITIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 2, 5 * 2 = 10
                    amount: vec![coin(10u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                LARGE_POSITIVE_TICK,
                0,
                OrderDirection::Bid,
                sender.clone(),
                Uint128::from(5u128),
                decimal256_from_u128(5u128),
                None,
            )),
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: valid two-step partial claim (large positive tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_POSITIVE_TICK,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
                // Claim the first partial fill
                OrderOperation::Claim((LARGE_POSITIVE_TICK, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(10u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_POSITIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    // Tick price = 2, 5 * 2 = 10
                    amount: vec![coin(10u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        // All Large Negative Tick orders operate on a tick price of 0.5
        ClaimOrderTestCase {
            name: "BID: valid basic full claim (large negative tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_NEGATIVE_TICK,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(50u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_NEGATIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(50u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: valid basic partial claim (large negative tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_NEGATIVE_TICK,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_NEGATIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: Some(LimitOrder::new(
                LARGE_NEGATIVE_TICK,
                0,
                OrderDirection::Bid,
                sender.clone(),
                Uint128::from(50u128),
                decimal256_from_u128(50u128),
                None,
            )),
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: valid two-step partial claim (large negative tick)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    LARGE_NEGATIVE_TICK,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
                // Claim the first partial fill
                OrderOperation::Claim((LARGE_NEGATIVE_TICK, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: LARGE_NEGATIVE_TICK,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "BID: full claim with a previous cancellation",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::Cancel((valid_tick_id, 0)),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Ask,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(100u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: None,
        },
        ClaimOrderTestCase {
            name: "invalid tick id",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(5u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 0,

            tick_id: 1,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: Some(ContractError::InvalidTickId { tick_id: 1 }),
        },
        ClaimOrderTestCase {
            name: "invalid order id",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(5u128),
                    OrderDirection::Bid,
                    Addr::unchecked("buyer"),
                )),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: Some(ContractError::OrderNotFound {
                tick_id: valid_tick_id,
                order_id: 1,
            }),
        },
        ClaimOrderTestCase {
            name: "invalid order id (cancelled order)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::Cancel((valid_tick_id, 0)),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: Some(ContractError::OrderNotFound {
                tick_id: valid_tick_id,
                order_id: 0,
            }),
        },
        ClaimOrderTestCase {
            name: "zero claim amount",
            sender: sender.clone(),
            operations: vec![OrderOperation::PlaceLimit(LimitOrder::new(
                valid_tick_id,
                0,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(10u128),
                Decimal256::zero(),
                None,
            ))],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: Some(ContractError::ZeroClaim),
        },
        ClaimOrderTestCase {
            name: "zero claim amount (tick etas < order etas)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: Some(ContractError::ZeroClaim),
        },
        ClaimOrderTestCase {
            name: "zero claim amount (cancelled order larger etas than order)",
            sender: sender.clone(),
            operations: vec![
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(10u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::Cancel((valid_tick_id, 1)),
            ],
            order_id: 0,

            tick_id: valid_tick_id,
            expected_bank_msg: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(5u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_bounty_msg: None,
            expected_order_state: None,
            expected_error: Some(ContractError::ZeroClaim),
        },
    ];

    for test in test_cases {
        println!("Running test: {}", test.name);
        // Test Setup
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = mock_info("sender", &[]);
        create_orderbook(
            deps.as_mut(),
            quote_denom.to_string(),
            base_denom.to_string(),
        )
        .unwrap();

        // Run setup operations
        for operation in test.operations {
            operation
                .run(deps.as_mut(), env.clone(), info.clone())
                .unwrap();
        }

        // Claim designated order
        let res = claim_order(
            deps.as_mut().storage,
            test.sender,
            test.tick_id,
            test.order_id,
        );

        if let Some(err) = test.expected_error {
            assert_eq!(res, Err(err), "{}", format_test_name(test.name));
            continue;
        }

        let res = res.unwrap();

        // Assert that the generated bank and bounty messages are as expected
        assert_eq!(
            res.1[0],
            test.expected_bank_msg,
            "{}",
            format_test_name(test.name)
        );
        if let Some(expected_bounty_msg) = test.expected_bounty_msg {
            // Bounty message expected
            assert_eq!((res.1).len(), 2, "{}", format_test_name(test.name));
            assert_eq!(
                res.1[1],
                expected_bounty_msg,
                "{}",
                format_test_name(test.name)
            );
        } else {
            // No bounty message expected
            assert_eq!((res.1).len(), 1, "{}", format_test_name(test.name));
        }

        // Check order in state
        let maybe_order = orders()
            .may_load(deps.as_ref().storage, &(test.tick_id, test.order_id))
            .unwrap();
        // Order in state may have been removed
        assert_eq!(
            maybe_order,
            test.expected_order_state,
            "{}",
            format_test_name(test.name)
        );
    }
}

struct MovingClaimOrderTestCase {
    name: &'static str,
    operations: Vec<OrderOperation>,
    sender: Addr,

    tick_id: i64,
    order_id: u64,
    expected_output: SubMsg,
    expected_order_state: Option<LimitOrder>,
    expected_error: Option<ContractError>,
}

#[test]
fn test_claim_order_moving_tick() {
    let valid_tick_id = 0;
    let quote_denom = "quote";
    let base_denom = "base";
    let sender = Addr::unchecked("sender");
    let test_cases: Vec<MovingClaimOrderTestCase> = vec![
        MovingClaimOrderTestCase {
            name: "ASK: single tick movement full claim",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediately fully fill
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Place limit in opposite direction from first order on same tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(50u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Ensure no errors on claiming first order
                OrderOperation::Claim((valid_tick_id, 0)),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(50u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: None,
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "ASK: single tick movement partial claim",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediately fully fill
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Place limit in opposite direction from first order on same tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Ensure no errors on claiming first order
                OrderOperation::Claim((valid_tick_id, 0)),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                1,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(25u128),
                decimal256_from_u128(25u128),
                None,
            )),
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "ASK: single tick movement partial claim with cancellation",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediately fully fill
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Place temporary order to be cancelled
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(25u128),
                    Decimal256::zero(),
                    None,
                )),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    2,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                // Cancel temporary order
                OrderOperation::Cancel((valid_tick_id, 1)),
                // Partially fill order to be claimed
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Ensure no errors on claiming first order
                OrderOperation::Claim((valid_tick_id, 0)),
            ],
            order_id: 2,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                2,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(25u128),
                decimal256_from_u128(50u128),
                None,
            )),
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name:
                "ASK: single tick movement partial claim with cancellation from previous direction",
            sender: sender.clone(),
            operations: vec![
                // Place order in opposite direction from claimed order
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                // Immediately cancel it
                OrderOperation::Cancel((valid_tick_id, 0)),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                // Partially fill order to be claimed
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                1,
                OrderDirection::Ask,
                sender.clone(),
                Uint128::from(25u128),
                decimal256_from_u128(25u128),
                None,
            )),
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "ASK: returning tick movement full claim",
            sender: sender.clone(),
            operations: vec![
                // Place order in opposite direction of order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill order to move tick
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                // Fill order to move tick
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(50u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Place new order in opposite direction to order that's being claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    2,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                // Full fill new order
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Ensure no errors on claiming orders for opposite directions
                OrderOperation::Claim((valid_tick_id, 0)),
                OrderOperation::Claim((valid_tick_id, 2)),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(50u128, quote_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: None,
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "BID: single tick movement full claim",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediately fully fill to move tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(50u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Ensure no errors on claiming first order
                OrderOperation::Claim((valid_tick_id, 0)),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(50u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: None,
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "BID: single tick movement partial claim",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediately fully fill
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Ensure no errors on claiming first order
                OrderOperation::Claim((valid_tick_id, 0)),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                1,
                OrderDirection::Bid,
                sender.clone(),
                Uint128::from(25u128),
                decimal256_from_u128(25u128),
                None,
            )),
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "BID: single tick movement partial claim with cancellation",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediately fully fill to move tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Place temporary order to be cancelled
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(25u128),
                    Decimal256::zero(),
                    None,
                )),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    2,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                // Cancel temporary order
                OrderOperation::Cancel((valid_tick_id, 1)),
                // Partially fill order to be claimed
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Ensure no errors on claiming first order
                OrderOperation::Claim((valid_tick_id, 0)),
            ],
            order_id: 2,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                2,
                OrderDirection::Bid,
                sender.clone(),
                Uint128::from(25u128),
                decimal256_from_u128(50u128),
                None,
            )),
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name:
                "BID: single tick movement partial claim with cancellation from previous direction",
            sender: sender.clone(),
            operations: vec![
                // Place order in opposite direction of order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                // Immediately cancel the order
                OrderOperation::Cancel((valid_tick_id, 0)),
                // Place order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                // Partially fill the order
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(25u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(25u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: Some(LimitOrder::new(
                valid_tick_id,
                1,
                OrderDirection::Bid,
                sender.clone(),
                Uint128::from(25u128),
                decimal256_from_u128(25u128),
                None,
            )),
            expected_error: None,
        },
        MovingClaimOrderTestCase {
            name: "BID: returning tick movement full claim",
            sender: sender.clone(),
            operations: vec![
                // Place order and immediatelly fully fill to move tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    0,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Place order to be claimed and fully fill to move tick
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    1,
                    OrderDirection::Bid,
                    sender.clone(),
                    Uint128::from(50u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(50u128),
                    OrderDirection::Ask,
                    sender.clone(),
                )),
                // Place order in opposite direction from order to be claimed
                OrderOperation::PlaceLimit(LimitOrder::new(
                    valid_tick_id,
                    2,
                    OrderDirection::Ask,
                    sender.clone(),
                    Uint128::from(100u128),
                    Decimal256::zero(),
                    None,
                )),
                OrderOperation::RunMarket(MarketOrder::new(
                    Uint128::from(100u128),
                    OrderDirection::Bid,
                    sender.clone(),
                )),
                // Ensure no errors on claiming orders for opposite direction
                OrderOperation::Claim((valid_tick_id, 0)),
                OrderOperation::Claim((valid_tick_id, 2)),
            ],
            order_id: 1,

            tick_id: valid_tick_id,
            expected_output: SubMsg::reply_on_error(
                BankMsg::Send {
                    to_address: sender.to_string(),
                    amount: vec![coin(50u128, base_denom)],
                },
                REPLY_ID_CLAIM,
            ),
            expected_order_state: None,
            expected_error: None,
        },
    ];

    for test in test_cases {
        // Test Setup
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = mock_info("sender", &[]);
        create_orderbook(
            deps.as_mut(),
            quote_denom.to_string(),
            base_denom.to_string(),
        )
        .unwrap();

        // Run setup operations
        for operation in test.operations {
            operation
                .run(deps.as_mut(), env.clone(), info.clone())
                .unwrap();
        }

        // Claim designated order
        let res = claim_order(
            deps.as_mut().storage,
            test.sender,
            test.tick_id,
            test.order_id,
        );

        if let Some(err) = test.expected_error {
            assert_eq!(res, Err(err), "{}", format_test_name(test.name));
            continue;
        }

        let res = res.unwrap();

        // Assert that the generated bank message is as expected
        assert_eq!(
            res.1[0],
            test.expected_output,
            "{}",
            format_test_name(test.name)
        );

        // Check order in state
        let maybe_order = orders()
            .may_load(deps.as_ref().storage, &(test.tick_id, test.order_id))
            .unwrap();
        // Order in state may have been removed
        assert_eq!(
            maybe_order,
            test.expected_order_state,
            "{}",
            format_test_name(test.name)
        );
    }
}
