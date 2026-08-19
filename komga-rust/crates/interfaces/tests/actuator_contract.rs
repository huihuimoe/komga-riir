use komga_interfaces::contracts::actuator::{
    ActuatorBuildDto, ActuatorDiskSpaceDetailsDto, ActuatorHealthDto, ActuatorHealthStatusDto,
    ActuatorInfoDto, ActuatorMemoryPoolDto, ActuatorMetricDetailDto, ActuatorMetricMeasurementDto,
    ActuatorMetricsIndexDto, ActuatorOsDto, ActuatorProcessDto, ActuatorProcessMemoryDto,
    ActuatorRootDto,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn actuator_health_dto_omits_components_when_details_are_hidden() {
    let payload = serde_json::to_value(ActuatorHealthDto {
        status: ActuatorHealthStatusDto::Up,
        components: None,
    })
    .expect("actuator health should serialize");

    assert_eq!(payload, json!({ "status": "UP" }));
}

#[test]
fn actuator_dtos_preserve_spring_field_names_and_optional_fields() {
    let root = serde_json::to_value(ActuatorRootDto {
        links: BTreeMap::from([(
            "health".to_string(),
            komga_interfaces::contracts::actuator::ActuatorLinkDto {
                href: "/actuator/health".to_string(),
                templated: false,
            },
        )]),
    })
    .expect("actuator root should serialize");
    assert_eq!(
        root,
        json!({ "_links": { "health": { "href": "/actuator/health", "templated": false } } })
    );

    let info = serde_json::to_value(ActuatorInfoDto {
        build: ActuatorBuildDto {
            artifact: "komga".to_string(),
            name: "komga-rust".to_string(),
            version: "1.0.0".to_string(),
            group: "huihuimoe".to_string(),
        },
        os: ActuatorOsDto {
            name: "Linux".to_string(),
            arch: "x86_64".to_string(),
            version: None,
        },
        process: ActuatorProcessDto {
            pid: 1,
            parent_pid: None,
            cpus: 1,
            virtual_threads: false,
            memory: ActuatorProcessMemoryDto {
                heap: ActuatorMemoryPoolDto {
                    used: 1,
                    committed: 2,
                    max: 3,
                },
                non_heap: ActuatorMemoryPoolDto {
                    used: 4,
                    committed: 5,
                    max: 6,
                },
            },
        },
        git: None,
    })
    .expect("actuator info should serialize");
    assert_eq!(
        info["build"],
        json!({ "artifact": "komga", "name": "komga-rust", "version": "1.0.0", "group": "huihuimoe" })
    );
    assert_eq!(info["os"], json!({ "name": "Linux", "arch": "x86_64" }));
    assert!(info.get("git").is_none());

    let disk = serde_json::to_value(ActuatorDiskSpaceDetailsDto {
        total: None,
        free: None,
        threshold: 10,
        path: "/".to_string(),
    })
    .expect("disk details should serialize");
    assert_eq!(disk, json!({ "threshold": 10, "path": "/" }));

    let metric = serde_json::to_value(ActuatorMetricDetailDto {
        name: "disk.free".to_string(),
        description: "Usable disk space".to_string(),
        base_unit: None,
        measurements: vec![ActuatorMetricMeasurementDto {
            statistic: "VALUE".to_string(),
            value: 42.0,
        }],
        available_tags: vec![],
    })
    .expect("metric detail should serialize");
    assert_eq!(
        metric,
        json!({
            "name": "disk.free",
            "description": "Usable disk space",
            "measurements": [{ "statistic": "VALUE", "value": 42.0 }],
            "availableTags": []
        })
    );

    let metrics = serde_json::to_value(ActuatorMetricsIndexDto {
        names: vec!["disk.free".to_string()],
    })
    .expect("metric index should serialize");
    assert_eq!(metrics, json!({ "names": ["disk.free"] }));
}
