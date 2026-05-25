use super::*;

#[test]
fn test_do_parse_version() {
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("1.2.3").unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("v1.2.3").unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("v1.2.3-alpha").unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("1.25.0+xyz").unwrap(),
        vec![1, 25, 0]
    );
    K8sJobMachine::<state::Initialized>::do_parse_version("invalid").unwrap_err();
}

#[test]
fn test_do_check_version() {
    K8sJobMachine::<state::Initialized>::do_check_version("1.25.0", "1.24.0").unwrap();
    K8sJobMachine::<state::Initialized>::do_check_version("1.25.0", "1.25.0").unwrap();
    K8sJobMachine::<state::Initialized>::do_check_version("1.25.1", "1.25.0").unwrap();

    let err =
        K8sJobMachine::<state::Initialized>::do_check_version("1.24.0", "1.25.0").unwrap_err();
    assert!(err.to_string().contains("less than required version"));
}
