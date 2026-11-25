//! Declarative macro that defines `TaskKind` and derives its Kotlin class metadata,
//! deduplication ID, series grouping, and serde JSON codecs from a single table.

macro_rules! opt_expr {
    () => {
        None
    };
    ($expr:expr) => {
        Some($expr)
    };
}
pub(super) use opt_expr;

macro_rules! encode_wire {
    (
        $priority:ident;
        fields: [ $( $(#[serde($($sattr:tt)*)])* $field:ident : $fty:ty ),+ ];
        extra: [ $( $extra:ident : $extra_ty:ty = $extra_val:expr ),* ];
    ) => {{
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            $(
                $(#[serde($($sattr)*)])*
                $field: &'a $fty,
            )+
            $( $extra: $extra_ty, )*
            priority: i32,
        }
        serde_json::to_string(&Wire {
            $( $field, )+
            $( $extra: $extra_val, )*
            priority: $priority,
        })
    }};
    (
        $priority:ident;
        fields: [];
        extra: [ $( $extra:ident : $extra_ty:ty = $extra_val:expr ),* ];
    ) => {{
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            $( $extra: $extra_ty, )*
            priority: i32,
        }
        serde_json::to_string(&Wire {
            $( $extra: $extra_val, )*
            priority: $priority,
        })
    }};
}
pub(super) use encode_wire;

macro_rules! decode_wire {
    (
        $variant:ident, $raw:ident, $invalid:ident;
        fields: [ $( $(#[serde($($sattr:tt)*)])* $field:ident : $fty:ty ),+ ];
    ) => {{
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            $(
                $(#[serde($($sattr)*)])*
                $field: $fty,
            )+
            priority: i32,
        }
        let Wire { $( $field, )+ priority } = serde_json::from_str($raw).map_err($invalid)?;
        Ok(Task {
            kind: TaskKind::$variant { $( $field ),+ },
            priority,
        })
    }};
    (
        $variant:ident, $raw:ident, $invalid:ident;
        fields: [];
    ) => {{
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            priority: i32,
        }
        let Wire { priority } = serde_json::from_str($raw).map_err($invalid)?;
        Ok(Task {
            kind: TaskKind::$variant,
            priority,
        })
    }};
}
pub(super) use decode_wire;

/// Defines `TaskKind`, `Task` constructors (defaulting to [`DEFAULT_PRIORITY`]), and their
/// single-source-of-truth metadata and serde codecs.
///
/// Each variant declares its constructor name, fields (with optional `#[serde(...)]` field
/// attributes for the persisted Jackson wire format), `id` expression, optional `group`
/// expression returning `&SeriesId`, and optional `wire_extra` fields written during serialization.
macro_rules! define_tasks {
    (
        $(
            $(#[$meta:meta])*
            $variant:ident ( $ctor:ident ) $( {
                $(
                    $(#[serde($($sattr:tt)*)])*
                    $field:ident : $fty:ty
                ),+ $(,)?
            } )? => {
                id: $id_expr:expr,
                $( group: $group_expr:expr, )?
                $( wire_extra: { $( $extra:ident : $extra_ty:ty = $extra_val:expr ),+ $(,)? }, )?
            }
        ),+ $(,)?
    ) => {
        /// Class names this build can execute; rows of other classes stay queued until a build
        /// that runs them, or the Kotlin server, claims them.
        pub(crate) const SUPPORTED_CLASSES: &[&str] = &[
            $(
                concat!("org.gotson.komga.application.tasks.Task$", stringify!($variant)),
            )+
        ];

        /// Variants carrying a `series_id` are grouped by series, so that tasks of one series
        /// never run concurrently, except `DeleteSeries` like in Kotlin.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum TaskKind {
            $(
                $(#[$meta])*
                $variant $( { $($field: $fty),+ } )?,
            )+
        }

        impl Task {
            $(
                $(#[$meta])*
                pub fn $ctor( $( $( $field: $fty ),+ )? ) -> Self {
                    Self {
                        kind: TaskKind::$variant $( { $( $field ),+ } )?,
                        priority: DEFAULT_PRIORITY,
                    }
                }
            )+
        }

        impl TaskKind {
            /// The deduplication key: submitting a task with a queued unique ID replaces that task.
            #[must_use]
            pub(crate) fn unique_id(&self) -> String {
                match self {
                    $(
                        Self::$variant $( { $($field),+ } )? => {
                            $($(let _ = $field;)+)?
                            $id_expr
                        }
                    )+
                }
            }

            pub(crate) fn simple_type(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant { .. } => stringify!($variant),
                    )+
                }
            }

            pub(crate) fn group_id(&self) -> Option<&SeriesId> {
                match self {
                    $(
                        Self::$variant $( { $($field),+ } )? => {
                            $($(let _ = $field;)+)?
                            macros::opt_expr!($($group_expr)?)
                        }
                    )+
                }
            }

            pub(crate) fn encode_payload(&self, priority: i32) -> serde_json::Result<String> {
                match self {
                    $(
                        Self::$variant $( { $($field),+ } )? => macros::encode_wire!(
                            priority;
                            fields: [ $($($(#[serde($($sattr)*)])* $field : $fty),+)? ];
                            extra: [ $($($extra : $extra_ty = $extra_val),+)? ];
                        ),
                    )+
                }
            }

            pub(crate) fn decode_task(class: &str, raw: &str) -> Result<Task, TaskDecodeError> {
                let invalid = |source| TaskDecodeError::Payload {
                    class: class.to_owned(),
                    source,
                };
                let Some(simple) = class.strip_prefix(CLASS_PREFIX) else {
                    return Err(TaskDecodeError::UnsupportedClass(class.to_owned()));
                };
                match simple {
                    $(
                        stringify!($variant) => macros::decode_wire!(
                            $variant, raw, invalid;
                            fields: [ $($($(#[serde($($sattr)*)])* $field : $fty),+)? ];
                        ),
                    )+
                    _ => Err(TaskDecodeError::UnsupportedClass(class.to_owned())),
                }
            }
        }
    };
}
pub(super) use define_tasks;
