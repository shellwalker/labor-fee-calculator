use labor_fee_core::{forward, Config, SurchargeItem};
use rust_decimal_macros::dec;

#[test]
fn duplicate_surcharge_names_ignore_surrounding_whitespace() {
    let mut cfg = Config::default();
    cfg.surcharge.items = vec![
        SurchargeItem::new("附加项目", dec!(0.01), true),
        SurchargeItem::new(" 附加项目 ", dec!(0.02), false),
    ];
    assert!(cfg.validate().is_err());
    assert!(forward(&cfg, dec!(1000)).is_err());
}

#[test]
fn legacy_total_rounding_is_ignored_and_not_serialized() {
    let text = labor_fee_core::DEFAULT_CONFIG_TOML
        .replace("[rounding]", "[rounding]\nsurcharge = \"total\"");
    let cfg = Config::from_toml_str(&text).unwrap();
    let encoded = toml::to_string(&cfg.rounding).unwrap();
    assert!(!encoded.contains("surcharge"));
}

#[test]
fn shared_amount_parser_accepts_all_frontend_formats() {
    for text in [" ¥8,397.00元 ", "1.25e3", "1，250", " 1 250 "] {
        assert!(labor_fee_core::parse_amount(text).is_ok(), "{text}");
    }
    assert_eq!(labor_fee_core::parse_amount("1.25e3").unwrap(), dec!(1250));
    for text in ["", "元 ¥", "abc", "1e999", "￥８"] {
        assert!(labor_fee_core::parse_amount(text).is_err());
    }
}

#[test]
fn explicit_group_reference_only_changes_vat_and_surcharges() {
    let cfg = Config::default();
    let normal = forward(&cfg, dec!(600)).unwrap();
    let grouped = labor_fee_core::forward_with_vat_reference(&cfg, dec!(600), dec!(1200)).unwrap();
    assert!(normal.vat_exempt);
    assert!(!grouped.vat_exempt);
    assert_eq!(grouped.vat, dec!(6));
    assert_eq!(grouped.iit, normal.iit);
    assert_eq!(grouped.take_home, normal.take_home);
}

#[test]
fn grouped_total_cost_inverts_the_same_vat_regime() {
    let cfg = Config::default();
    for reference in [dec!(0), dec!(1200)] {
        for net in [dec!(600), dec!(999.99), dec!(1000), dec!(1050.50)] {
            let result = labor_fee_core::forward_with_vat_reference(&cfg, net, reference).unwrap();
            assert_eq!(
                labor_fee_core::from_total_cost_with_vat_reference(
                    &cfg,
                    result.total_cost,
                    reference
                )
                .unwrap(),
                result
            );
        }
    }
}
