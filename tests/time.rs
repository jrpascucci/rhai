#![cfg(not(feature = "no_stdlib"))]

use rhai::{Engine, EvalAltResult, INT};

#[cfg(not(feature = "no_float"))]
use rhai::FLOAT;

#[test]
fn test_timestamp() -> Result<(), EvalAltResult> {
    let engine = Engine::new();

    assert_eq!(engine.eval::<String>("type_of(timestamp())")?, "timestamp");

    #[cfg(not(feature = "no_float"))]
    assert!(
        engine.eval::<FLOAT>(
            r#"
                let time = timestamp();
                let x = 10_000;
                while x > 0 { x -= 1; }
                elapsed(time)
    "#
        )? < 10.0
    );

    #[cfg(feature = "no_float")]
    assert!(
        engine.eval::<INT>(
            r#"
                let time = timestamp();
                let x = 10_000;
                while x > 0 { x -= 1; }
                elapsed(time)
    "#
        )? < 10
    );

    Ok(())
}
