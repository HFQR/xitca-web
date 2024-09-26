/// A SQLSTATE error code
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct SqlState(Inner);

impl SqlState {
    /// Creates a `SqlState` from its error code.
    pub fn from_code(s: &str) -> SqlState {
        match Self::state(s) {
            Some(state) => state,
            None => SqlState(Inner::Other(s.into())),
        }
    }

    /// Returns the error code corresponding to the `SqlState`.
    pub fn code(&self) -> &str {
        match &self.0 {
            Inner::E00000 => "00000",
            Inner::E01000 => "01000",
            Inner::E0100C => "0100C",
            Inner::E01008 => "01008",
            Inner::E01003 => "01003",
            Inner::E01007 => "01007",
            Inner::E01006 => "01006",
            Inner::E01004 => "01004",
            Inner::E01P01 => "01P01",
            Inner::E02000 => "02000",
            Inner::E02001 => "02001",
            Inner::E03000 => "03000",
            Inner::E08000 => "08000",
            Inner::E08003 => "08003",
            Inner::E08006 => "08006",
            Inner::E08001 => "08001",
            Inner::E08004 => "08004",
            Inner::E08007 => "08007",
            Inner::E08P01 => "08P01",
            Inner::E09000 => "09000",
            Inner::E0A000 => "0A000",
            Inner::E0B000 => "0B000",
            Inner::E0F000 => "0F000",
            Inner::E0F001 => "0F001",
            Inner::E0L000 => "0L000",
            Inner::E0LP01 => "0LP01",
            Inner::E0P000 => "0P000",
            Inner::E0Z000 => "0Z000",
            Inner::E0Z002 => "0Z002",
            Inner::E20000 => "20000",
            Inner::E21000 => "21000",
            Inner::E22000 => "22000",
            Inner::E2202E => "2202E",
            Inner::E22021 => "22021",
            Inner::E22008 => "22008",
            Inner::E22012 => "22012",
            Inner::E22005 => "22005",
            Inner::E2200B => "2200B",
            Inner::E22022 => "22022",
            Inner::E22015 => "22015",
            Inner::E2201E => "2201E",
            Inner::E22014 => "22014",
            Inner::E22016 => "22016",
            Inner::E2201F => "2201F",
            Inner::E2201G => "2201G",
            Inner::E22018 => "22018",
            Inner::E22007 => "22007",
            Inner::E22019 => "22019",
            Inner::E2200D => "2200D",
            Inner::E22025 => "22025",
            Inner::E22P06 => "22P06",
            Inner::E22010 => "22010",
            Inner::E22023 => "22023",
            Inner::E22013 => "22013",
            Inner::E2201B => "2201B",
            Inner::E2201W => "2201W",
            Inner::E2201X => "2201X",
            Inner::E2202H => "2202H",
            Inner::E2202G => "2202G",
            Inner::E22009 => "22009",
            Inner::E2200C => "2200C",
            Inner::E2200G => "2200G",
            Inner::E22004 => "22004",
            Inner::E22002 => "22002",
            Inner::E22003 => "22003",
            Inner::E2200H => "2200H",
            Inner::E22026 => "22026",
            Inner::E22001 => "22001",
            Inner::E22011 => "22011",
            Inner::E22027 => "22027",
            Inner::E22024 => "22024",
            Inner::E2200F => "2200F",
            Inner::E22P01 => "22P01",
            Inner::E22P02 => "22P02",
            Inner::E22P03 => "22P03",
            Inner::E22P04 => "22P04",
            Inner::E22P05 => "22P05",
            Inner::E2200L => "2200L",
            Inner::E2200M => "2200M",
            Inner::E2200N => "2200N",
            Inner::E2200S => "2200S",
            Inner::E2200T => "2200T",
            Inner::E22030 => "22030",
            Inner::E22031 => "22031",
            Inner::E22032 => "22032",
            Inner::E22033 => "22033",
            Inner::E22034 => "22034",
            Inner::E22035 => "22035",
            Inner::E22036 => "22036",
            Inner::E22037 => "22037",
            Inner::E22038 => "22038",
            Inner::E22039 => "22039",
            Inner::E2203A => "2203A",
            Inner::E2203B => "2203B",
            Inner::E2203C => "2203C",
            Inner::E2203D => "2203D",
            Inner::E2203E => "2203E",
            Inner::E2203F => "2203F",
            Inner::E2203G => "2203G",
            Inner::E23000 => "23000",
            Inner::E23001 => "23001",
            Inner::E23502 => "23502",
            Inner::E23503 => "23503",
            Inner::E23505 => "23505",
            Inner::E23514 => "23514",
            Inner::E23P01 => "23P01",
            Inner::E24000 => "24000",
            Inner::E25000 => "25000",
            Inner::E25001 => "25001",
            Inner::E25002 => "25002",
            Inner::E25008 => "25008",
            Inner::E25003 => "25003",
            Inner::E25004 => "25004",
            Inner::E25005 => "25005",
            Inner::E25006 => "25006",
            Inner::E25007 => "25007",
            Inner::E25P01 => "25P01",
            Inner::E25P02 => "25P02",
            Inner::E25P03 => "25P03",
            Inner::E26000 => "26000",
            Inner::E27000 => "27000",
            Inner::E28000 => "28000",
            Inner::E28P01 => "28P01",
            Inner::E2B000 => "2B000",
            Inner::E2BP01 => "2BP01",
            Inner::E2D000 => "2D000",
            Inner::E2F000 => "2F000",
            Inner::E2F005 => "2F005",
            Inner::E2F002 => "2F002",
            Inner::E2F003 => "2F003",
            Inner::E2F004 => "2F004",
            Inner::E34000 => "34000",
            Inner::E38000 => "38000",
            Inner::E38001 => "38001",
            Inner::E38002 => "38002",
            Inner::E38003 => "38003",
            Inner::E38004 => "38004",
            Inner::E39000 => "39000",
            Inner::E39001 => "39001",
            Inner::E39004 => "39004",
            Inner::E39P01 => "39P01",
            Inner::E39P02 => "39P02",
            Inner::E39P03 => "39P03",
            Inner::E3B000 => "3B000",
            Inner::E3B001 => "3B001",
            Inner::E3D000 => "3D000",
            Inner::E3F000 => "3F000",
            Inner::E40000 => "40000",
            Inner::E40002 => "40002",
            Inner::E40001 => "40001",
            Inner::E40003 => "40003",
            Inner::E40P01 => "40P01",
            Inner::E42000 => "42000",
            Inner::E42601 => "42601",
            Inner::E42501 => "42501",
            Inner::E42846 => "42846",
            Inner::E42803 => "42803",
            Inner::E42P20 => "42P20",
            Inner::E42P19 => "42P19",
            Inner::E42830 => "42830",
            Inner::E42602 => "42602",
            Inner::E42622 => "42622",
            Inner::E42939 => "42939",
            Inner::E42804 => "42804",
            Inner::E42P18 => "42P18",
            Inner::E42P21 => "42P21",
            Inner::E42P22 => "42P22",
            Inner::E42809 => "42809",
            Inner::E428C9 => "428C9",
            Inner::E42703 => "42703",
            Inner::E42883 => "42883",
            Inner::E42P01 => "42P01",
            Inner::E42P02 => "42P02",
            Inner::E42704 => "42704",
            Inner::E42701 => "42701",
            Inner::E42P03 => "42P03",
            Inner::E42P04 => "42P04",
            Inner::E42723 => "42723",
            Inner::E42P05 => "42P05",
            Inner::E42P06 => "42P06",
            Inner::E42P07 => "42P07",
            Inner::E42712 => "42712",
            Inner::E42710 => "42710",
            Inner::E42702 => "42702",
            Inner::E42725 => "42725",
            Inner::E42P08 => "42P08",
            Inner::E42P09 => "42P09",
            Inner::E42P10 => "42P10",
            Inner::E42611 => "42611",
            Inner::E42P11 => "42P11",
            Inner::E42P12 => "42P12",
            Inner::E42P13 => "42P13",
            Inner::E42P14 => "42P14",
            Inner::E42P15 => "42P15",
            Inner::E42P16 => "42P16",
            Inner::E42P17 => "42P17",
            Inner::E44000 => "44000",
            Inner::E53000 => "53000",
            Inner::E53100 => "53100",
            Inner::E53200 => "53200",
            Inner::E53300 => "53300",
            Inner::E53400 => "53400",
            Inner::E54000 => "54000",
            Inner::E54001 => "54001",
            Inner::E54011 => "54011",
            Inner::E54023 => "54023",
            Inner::E55000 => "55000",
            Inner::E55006 => "55006",
            Inner::E55P02 => "55P02",
            Inner::E55P03 => "55P03",
            Inner::E55P04 => "55P04",
            Inner::E57000 => "57000",
            Inner::E57014 => "57014",
            Inner::E57P01 => "57P01",
            Inner::E57P02 => "57P02",
            Inner::E57P03 => "57P03",
            Inner::E57P04 => "57P04",
            Inner::E57P05 => "57P05",
            Inner::E58000 => "58000",
            Inner::E58030 => "58030",
            Inner::E58P01 => "58P01",
            Inner::E58P02 => "58P02",
            Inner::E72000 => "72000",
            Inner::EF0000 => "F0000",
            Inner::EF0001 => "F0001",
            Inner::EHV000 => "HV000",
            Inner::EHV005 => "HV005",
            Inner::EHV002 => "HV002",
            Inner::EHV010 => "HV010",
            Inner::EHV021 => "HV021",
            Inner::EHV024 => "HV024",
            Inner::EHV007 => "HV007",
            Inner::EHV008 => "HV008",
            Inner::EHV004 => "HV004",
            Inner::EHV006 => "HV006",
            Inner::EHV091 => "HV091",
            Inner::EHV00B => "HV00B",
            Inner::EHV00C => "HV00C",
            Inner::EHV00D => "HV00D",
            Inner::EHV090 => "HV090",
            Inner::EHV00A => "HV00A",
            Inner::EHV009 => "HV009",
            Inner::EHV014 => "HV014",
            Inner::EHV001 => "HV001",
            Inner::EHV00P => "HV00P",
            Inner::EHV00J => "HV00J",
            Inner::EHV00K => "HV00K",
            Inner::EHV00Q => "HV00Q",
            Inner::EHV00R => "HV00R",
            Inner::EHV00L => "HV00L",
            Inner::EHV00M => "HV00M",
            Inner::EHV00N => "HV00N",
            Inner::EP0000 => "P0000",
            Inner::EP0001 => "P0001",
            Inner::EP0002 => "P0002",
            Inner::EP0003 => "P0003",
            Inner::EP0004 => "P0004",
            Inner::EXX000 => "XX000",
            Inner::EXX001 => "XX001",
            Inner::EXX002 => "XX002",
            Inner::Other(code) => code,
        }
    }

    #[inline(never)]
    fn state(key: &str) -> Option<Self> {
        let key = match key {
            "2F000" => SqlState::SQL_ROUTINE_EXCEPTION,
            "01008" => SqlState::WARNING_IMPLICIT_ZERO_BIT_PADDING,
            "42501" => SqlState::INSUFFICIENT_PRIVILEGE,
            "22000" => SqlState::DATA_EXCEPTION,
            "0100C" => SqlState::WARNING_DYNAMIC_RESULT_SETS_RETURNED,
            "2200N" => SqlState::INVALID_XML_CONTENT,
            "40001" => SqlState::T_R_SERIALIZATION_FAILURE,
            "28P01" => SqlState::INVALID_PASSWORD,
            "38000" => SqlState::EXTERNAL_ROUTINE_EXCEPTION,
            "25006" => SqlState::READ_ONLY_SQL_TRANSACTION,
            "2203D" => SqlState::TOO_MANY_JSON_ARRAY_ELEMENTS,
            "42P09" => SqlState::AMBIGUOUS_ALIAS,
            "F0000" => SqlState::CONFIG_FILE_ERROR,
            "42P18" => SqlState::INDETERMINATE_DATATYPE,
            "40002" => SqlState::T_R_INTEGRITY_CONSTRAINT_VIOLATION,
            "22009" => SqlState::INVALID_TIME_ZONE_DISPLACEMENT_VALUE,
            "42P08" => SqlState::AMBIGUOUS_PARAMETER,
            "08000" => SqlState::CONNECTION_EXCEPTION,
            "25P01" => SqlState::NO_ACTIVE_SQL_TRANSACTION,
            "22024" => SqlState::UNTERMINATED_C_STRING,
            "55000" => SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
            "25001" => SqlState::ACTIVE_SQL_TRANSACTION,
            "03000" => SqlState::SQL_STATEMENT_NOT_YET_COMPLETE,
            "42710" => SqlState::DUPLICATE_OBJECT,
            "2D000" => SqlState::INVALID_TRANSACTION_TERMINATION,
            "2200G" => SqlState::MOST_SPECIFIC_TYPE_MISMATCH,
            "22022" => SqlState::INDICATOR_OVERFLOW,
            "55006" => SqlState::OBJECT_IN_USE,
            "53200" => SqlState::OUT_OF_MEMORY,
            "22012" => SqlState::DIVISION_BY_ZERO,
            "P0002" => SqlState::NO_DATA_FOUND,
            "XX001" => SqlState::DATA_CORRUPTED,
            "22P05" => SqlState::UNTRANSLATABLE_CHARACTER,
            "40003" => SqlState::T_R_STATEMENT_COMPLETION_UNKNOWN,
            "22021" => SqlState::CHARACTER_NOT_IN_REPERTOIRE,
            "25000" => SqlState::INVALID_TRANSACTION_STATE,
            "42P15" => SqlState::INVALID_SCHEMA_DEFINITION,
            "0B000" => SqlState::INVALID_TRANSACTION_INITIATION,
            "22004" => SqlState::NULL_VALUE_NOT_ALLOWED,
            "42804" => SqlState::DATATYPE_MISMATCH,
            "42803" => SqlState::GROUPING_ERROR,
            "02001" => SqlState::NO_ADDITIONAL_DYNAMIC_RESULT_SETS_RETURNED,
            "25002" => SqlState::BRANCH_TRANSACTION_ALREADY_ACTIVE,
            "28000" => SqlState::INVALID_AUTHORIZATION_SPECIFICATION,
            "HV009" => SqlState::FDW_INVALID_USE_OF_NULL_POINTER,
            "22P01" => SqlState::FLOATING_POINT_EXCEPTION,
            "2B000" => SqlState::DEPENDENT_PRIVILEGE_DESCRIPTORS_STILL_EXIST,
            "42723" => SqlState::DUPLICATE_FUNCTION,
            "21000" => SqlState::CARDINALITY_VIOLATION,
            "0Z002" => SqlState::STACKED_DIAGNOSTICS_ACCESSED_WITHOUT_ACTIVE_HANDLER,
            "23505" => SqlState::UNIQUE_VIOLATION,
            "HV00J" => SqlState::FDW_OPTION_NAME_NOT_FOUND,
            "23P01" => SqlState::EXCLUSION_VIOLATION,
            "39P03" => SqlState::E_R_I_E_EVENT_TRIGGER_PROTOCOL_VIOLATED,
            "42P10" => SqlState::INVALID_COLUMN_REFERENCE,
            "2202H" => SqlState::INVALID_TABLESAMPLE_ARGUMENT,
            "55P04" => SqlState::UNSAFE_NEW_ENUM_VALUE_USAGE,
            "P0000" => SqlState::PLPGSQL_ERROR,
            "2F005" => SqlState::S_R_E_FUNCTION_EXECUTED_NO_RETURN_STATEMENT,
            "HV00M" => SqlState::FDW_UNABLE_TO_CREATE_REPLY,
            "0A000" => SqlState::FEATURE_NOT_SUPPORTED,
            "24000" => SqlState::INVALID_CURSOR_STATE,
            "25008" => SqlState::HELD_CURSOR_REQUIRES_SAME_ISOLATION_LEVEL,
            "01003" => SqlState::WARNING_NULL_VALUE_ELIMINATED_IN_SET_FUNCTION,
            "42712" => SqlState::DUPLICATE_ALIAS,
            "HV014" => SqlState::FDW_TOO_MANY_HANDLES,
            "58030" => SqlState::IO_ERROR,
            "2201W" => SqlState::INVALID_ROW_COUNT_IN_LIMIT_CLAUSE,
            "22033" => SqlState::INVALID_SQL_JSON_SUBSCRIPT,
            "2BP01" => SqlState::DEPENDENT_OBJECTS_STILL_EXIST,
            "HV005" => SqlState::FDW_COLUMN_NAME_NOT_FOUND,
            "25004" => SqlState::INAPPROPRIATE_ISOLATION_LEVEL_FOR_BRANCH_TRANSACTION,
            "54000" => SqlState::PROGRAM_LIMIT_EXCEEDED,
            "20000" => SqlState::CASE_NOT_FOUND,
            "2203G" => SqlState::SQL_JSON_ITEM_CANNOT_BE_CAST_TO_TARGET_TYPE,
            "22038" => SqlState::SINGLETON_SQL_JSON_ITEM_REQUIRED,
            "22007" => SqlState::INVALID_DATETIME_FORMAT,
            "08004" => SqlState::SQLSERVER_REJECTED_ESTABLISHMENT_OF_SQLCONNECTION,
            "2200H" => SqlState::SEQUENCE_GENERATOR_LIMIT_EXCEEDED,
            "HV00D" => SqlState::FDW_INVALID_OPTION_NAME,
            "P0004" => SqlState::ASSERT_FAILURE,
            "22018" => SqlState::INVALID_CHARACTER_VALUE_FOR_CAST,
            "0L000" => SqlState::INVALID_GRANTOR,
            "22P04" => SqlState::BAD_COPY_FILE_FORMAT,
            "22031" => SqlState::INVALID_ARGUMENT_FOR_SQL_JSON_DATETIME_FUNCTION,
            "01P01" => SqlState::WARNING_DEPRECATED_FEATURE,
            "0LP01" => SqlState::INVALID_GRANT_OPERATION,
            "58P02" => SqlState::DUPLICATE_FILE,
            "26000" => SqlState::INVALID_SQL_STATEMENT_NAME,
            "54001" => SqlState::STATEMENT_TOO_COMPLEX,
            "22010" => SqlState::INVALID_INDICATOR_PARAMETER_VALUE,
            "HV00C" => SqlState::FDW_INVALID_OPTION_INDEX,
            "22008" => SqlState::DATETIME_FIELD_OVERFLOW,
            "42P06" => SqlState::DUPLICATE_SCHEMA,
            "25007" => SqlState::SCHEMA_AND_DATA_STATEMENT_MIXING_NOT_SUPPORTED,
            "42P20" => SqlState::WINDOWING_ERROR,
            "HV091" => SqlState::FDW_INVALID_DESCRIPTOR_FIELD_IDENTIFIER,
            "HV021" => SqlState::FDW_INCONSISTENT_DESCRIPTOR_INFORMATION,
            "42702" => SqlState::AMBIGUOUS_COLUMN,
            "02000" => SqlState::NO_DATA,
            "54011" => SqlState::TOO_MANY_COLUMNS,
            "HV004" => SqlState::FDW_INVALID_DATA_TYPE,
            "01006" => SqlState::WARNING_PRIVILEGE_NOT_REVOKED,
            "42701" => SqlState::DUPLICATE_COLUMN,
            "08P01" => SqlState::PROTOCOL_VIOLATION,
            "42622" => SqlState::NAME_TOO_LONG,
            "P0003" => SqlState::TOO_MANY_ROWS,
            "22003" => SqlState::NUMERIC_VALUE_OUT_OF_RANGE,
            "42P03" => SqlState::DUPLICATE_CURSOR,
            "23001" => SqlState::RESTRICT_VIOLATION,
            "57000" => SqlState::OPERATOR_INTERVENTION,
            "22027" => SqlState::TRIM_ERROR,
            "42P12" => SqlState::INVALID_DATABASE_DEFINITION,
            "3B000" => SqlState::SAVEPOINT_EXCEPTION,
            "2201B" => SqlState::INVALID_REGULAR_EXPRESSION,
            "22030" => SqlState::DUPLICATE_JSON_OBJECT_KEY_VALUE,
            "2F004" => SqlState::S_R_E_READING_SQL_DATA_NOT_PERMITTED,
            "428C9" => SqlState::GENERATED_ALWAYS,
            "2200S" => SqlState::INVALID_XML_COMMENT,
            "22039" => SqlState::SQL_JSON_ARRAY_NOT_FOUND,
            "42809" => SqlState::WRONG_OBJECT_TYPE,
            "2201X" => SqlState::INVALID_ROW_COUNT_IN_RESULT_OFFSET_CLAUSE,
            "39001" => SqlState::E_R_I_E_INVALID_SQLSTATE_RETURNED,
            "25P02" => SqlState::IN_FAILED_SQL_TRANSACTION,
            "0P000" => SqlState::INVALID_ROLE_SPECIFICATION,
            "HV00N" => SqlState::FDW_UNABLE_TO_ESTABLISH_CONNECTION,
            "53100" => SqlState::DISK_FULL,
            "42601" => SqlState::SYNTAX_ERROR,
            "23000" => SqlState::INTEGRITY_CONSTRAINT_VIOLATION,
            "HV006" => SqlState::FDW_INVALID_DATA_TYPE_DESCRIPTORS,
            "HV00B" => SqlState::FDW_INVALID_HANDLE,
            "HV00Q" => SqlState::FDW_SCHEMA_NOT_FOUND,
            "01000" => SqlState::WARNING,
            "42883" => SqlState::UNDEFINED_FUNCTION,
            "57P01" => SqlState::ADMIN_SHUTDOWN,
            "22037" => SqlState::NON_UNIQUE_KEYS_IN_A_JSON_OBJECT,
            "00000" => SqlState::SUCCESSFUL_COMPLETION,
            "55P03" => SqlState::LOCK_NOT_AVAILABLE,
            "42P01" => SqlState::UNDEFINED_TABLE,
            "42830" => SqlState::INVALID_FOREIGN_KEY,
            "22005" => SqlState::ERROR_IN_ASSIGNMENT,
            "22025" => SqlState::INVALID_ESCAPE_SEQUENCE,
            "XX002" => SqlState::INDEX_CORRUPTED,
            "42P16" => SqlState::INVALID_TABLE_DEFINITION,
            "55P02" => SqlState::CANT_CHANGE_RUNTIME_PARAM,
            "22019" => SqlState::INVALID_ESCAPE_CHARACTER,
            "P0001" => SqlState::RAISE_EXCEPTION,
            "72000" => SqlState::SNAPSHOT_TOO_OLD,
            "42P11" => SqlState::INVALID_CURSOR_DEFINITION,
            "40P01" => SqlState::T_R_DEADLOCK_DETECTED,
            "57P02" => SqlState::CRASH_SHUTDOWN,
            "HV00A" => SqlState::FDW_INVALID_STRING_FORMAT,
            "2F002" => SqlState::S_R_E_MODIFYING_SQL_DATA_NOT_PERMITTED,
            "23503" => SqlState::FOREIGN_KEY_VIOLATION,
            "40000" => SqlState::TRANSACTION_ROLLBACK,
            "22032" => SqlState::INVALID_JSON_TEXT,
            "2202E" => SqlState::ARRAY_ELEMENT_ERROR,
            "42P19" => SqlState::INVALID_RECURSION,
            "42611" => SqlState::INVALID_COLUMN_DEFINITION,
            "42P13" => SqlState::INVALID_FUNCTION_DEFINITION,
            "25003" => SqlState::INAPPROPRIATE_ACCESS_MODE_FOR_BRANCH_TRANSACTION,
            "39P02" => SqlState::E_R_I_E_SRF_PROTOCOL_VIOLATED,
            "XX000" => SqlState::INTERNAL_ERROR,
            "08006" => SqlState::CONNECTION_FAILURE,
            "57P04" => SqlState::DATABASE_DROPPED,
            "42P07" => SqlState::DUPLICATE_TABLE,
            "22P03" => SqlState::INVALID_BINARY_REPRESENTATION,
            "22035" => SqlState::NO_SQL_JSON_ITEM,
            "42P14" => SqlState::INVALID_PSTATEMENT_DEFINITION,
            "01007" => SqlState::WARNING_PRIVILEGE_NOT_GRANTED,
            "38004" => SqlState::E_R_E_READING_SQL_DATA_NOT_PERMITTED,
            "42P21" => SqlState::COLLATION_MISMATCH,
            "0Z000" => SqlState::DIAGNOSTICS_EXCEPTION,
            "HV001" => SqlState::FDW_OUT_OF_MEMORY,
            "0F000" => SqlState::LOCATOR_EXCEPTION,
            "22013" => SqlState::INVALID_PRECEDING_OR_FOLLOWING_SIZE,
            "2201E" => SqlState::INVALID_ARGUMENT_FOR_LOG,
            "22011" => SqlState::SUBSTRING_ERROR,
            "42602" => SqlState::INVALID_NAME,
            "01004" => SqlState::WARNING_STRING_DATA_RIGHT_TRUNCATION,
            "42P02" => SqlState::UNDEFINED_PARAMETER,
            "2203C" => SqlState::SQL_JSON_OBJECT_NOT_FOUND,
            "HV002" => SqlState::FDW_DYNAMIC_PARAMETER_VALUE_NEEDED,
            "0F001" => SqlState::L_E_INVALID_SPECIFICATION,
            "58P01" => SqlState::UNDEFINED_FILE,
            "38001" => SqlState::E_R_E_CONTAINING_SQL_NOT_PERMITTED,
            "42703" => SqlState::UNDEFINED_COLUMN,
            "57P05" => SqlState::IDLE_SESSION_TIMEOUT,
            "57P03" => SqlState::CANNOT_CONNECT_NOW,
            "HV007" => SqlState::FDW_INVALID_COLUMN_NAME,
            "22014" => SqlState::INVALID_ARGUMENT_FOR_NTILE,
            "22P06" => SqlState::NONSTANDARD_USE_OF_ESCAPE_CHARACTER,
            "2203F" => SqlState::SQL_JSON_SCALAR_REQUIRED,
            "2200F" => SqlState::ZERO_LENGTH_CHARACTER_STRING,
            "09000" => SqlState::TRIGGERED_ACTION_EXCEPTION,
            "2201F" => SqlState::INVALID_ARGUMENT_FOR_POWER_FUNCTION,
            "08003" => SqlState::CONNECTION_DOES_NOT_EXIST,
            "38002" => SqlState::E_R_E_MODIFYING_SQL_DATA_NOT_PERMITTED,
            "F0001" => SqlState::LOCK_FILE_EXISTS,
            "42P22" => SqlState::INDETERMINATE_COLLATION,
            "2200C" => SqlState::INVALID_USE_OF_ESCAPE_CHARACTER,
            "2203E" => SqlState::TOO_MANY_JSON_OBJECT_MEMBERS,
            "23514" => SqlState::CHECK_VIOLATION,
            "22P02" => SqlState::INVALID_TEXT_REPRESENTATION,
            "54023" => SqlState::TOO_MANY_ARGUMENTS,
            "2200T" => SqlState::INVALID_XML_PROCESSING_INSTRUCTION,
            "22016" => SqlState::INVALID_ARGUMENT_FOR_NTH_VALUE,
            "25P03" => SqlState::IDLE_IN_TRANSACTION_SESSION_TIMEOUT,
            "3B001" => SqlState::S_E_INVALID_SPECIFICATION,
            "08001" => SqlState::SQLCLIENT_UNABLE_TO_ESTABLISH_SQLCONNECTION,
            "22036" => SqlState::NON_NUMERIC_SQL_JSON_ITEM,
            "3F000" => SqlState::INVALID_SCHEMA_NAME,
            "39P01" => SqlState::E_R_I_E_TRIGGER_PROTOCOL_VIOLATED,
            "22026" => SqlState::STRING_DATA_LENGTH_MISMATCH,
            "42P17" => SqlState::INVALID_OBJECT_DEFINITION,
            "22034" => SqlState::MORE_THAN_ONE_SQL_JSON_ITEM,
            "HV000" => SqlState::FDW_ERROR,
            "2200B" => SqlState::ESCAPE_CHARACTER_CONFLICT,
            "HV008" => SqlState::FDW_INVALID_COLUMN_NUMBER,
            "34000" => SqlState::INVALID_CURSOR_NAME,
            "2201G" => SqlState::INVALID_ARGUMENT_FOR_WIDTH_BUCKET_FUNCTION,
            "44000" => SqlState::WITH_CHECK_OPTION_VIOLATION,
            "HV010" => SqlState::FDW_FUNCTION_SEQUENCE_ERROR,
            "39004" => SqlState::E_R_I_E_NULL_VALUE_NOT_ALLOWED,
            "22001" => SqlState::STRING_DATA_RIGHT_TRUNCATION,
            "3D000" => SqlState::INVALID_CATALOG_NAME,
            "25005" => SqlState::NO_ACTIVE_SQL_TRANSACTION_FOR_BRANCH_TRANSACTION,
            "2200L" => SqlState::NOT_AN_XML_DOCUMENT,
            "27000" => SqlState::TRIGGERED_DATA_CHANGE_VIOLATION,
            "HV090" => SqlState::FDW_INVALID_STRING_LENGTH_OR_BUFFER_LENGTH,
            "42939" => SqlState::RESERVED_NAME,
            "58000" => SqlState::SYSTEM_ERROR,
            "2200M" => SqlState::INVALID_XML_DOCUMENT,
            "HV00L" => SqlState::FDW_UNABLE_TO_CREATE_EXECUTION,
            "57014" => SqlState::QUERY_CANCELED,
            "23502" => SqlState::NOT_NULL_VIOLATION,
            "22002" => SqlState::NULL_VALUE_NO_INDICATOR_PARAMETER,
            "HV00R" => SqlState::FDW_TABLE_NOT_FOUND,
            "HV00P" => SqlState::FDW_NO_SCHEMAS,
            "38003" => SqlState::E_R_E_PROHIBITED_SQL_STATEMENT_ATTEMPTED,
            "39000" => SqlState::EXTERNAL_ROUTINE_INVOCATION_EXCEPTION,
            "22015" => SqlState::INTERVAL_FIELD_OVERFLOW,
            "HV00K" => SqlState::FDW_REPLY_HANDLE,
            "HV024" => SqlState::FDW_INVALID_ATTRIBUTE_VALUE,
            "2200D" => SqlState::INVALID_ESCAPE_OCTET,
            "08007" => SqlState::TRANSACTION_RESOLUTION_UNKNOWN,
            "2F003" => SqlState::S_R_E_PROHIBITED_SQL_STATEMENT_ATTEMPTED,
            "42725" => SqlState::AMBIGUOUS_FUNCTION,
            "2203A" => SqlState::SQL_JSON_MEMBER_NOT_FOUND,
            "42846" => SqlState::CANNOT_COERCE,
            "42P04" => SqlState::DUPLICATE_DATABASE,
            "42000" => SqlState::SYNTAX_ERROR_OR_ACCESS_RULE_VIOLATION,
            "2203B" => SqlState::SQL_JSON_NUMBER_NOT_FOUND,
            "42P05" => SqlState::DUPLICATE_PSTATEMENT,
            "53300" => SqlState::TOO_MANY_CONNECTIONS,
            "53400" => SqlState::CONFIGURATION_LIMIT_EXCEEDED,
            "42704" => SqlState::UNDEFINED_OBJECT,
            "2202G" => SqlState::INVALID_TABLESAMPLE_REPEAT,
            "22023" => SqlState::INVALID_PARAMETER_VALUE,
            "53000" => SqlState::INSUFFICIENT_RESOURCES,
            _ => return None,
        };

        Some(key)
    }

    /// 00000
    pub const SUCCESSFUL_COMPLETION: SqlState = SqlState(Inner::E00000);

    /// 01000
    pub const WARNING: SqlState = SqlState(Inner::E01000);

    /// 0100C
    pub const WARNING_DYNAMIC_RESULT_SETS_RETURNED: SqlState = SqlState(Inner::E0100C);

    /// 01008
    pub const WARNING_IMPLICIT_ZERO_BIT_PADDING: SqlState = SqlState(Inner::E01008);

    /// 01003
    pub const WARNING_NULL_VALUE_ELIMINATED_IN_SET_FUNCTION: SqlState = SqlState(Inner::E01003);

    /// 01007
    pub const WARNING_PRIVILEGE_NOT_GRANTED: SqlState = SqlState(Inner::E01007);

    /// 01006
    pub const WARNING_PRIVILEGE_NOT_REVOKED: SqlState = SqlState(Inner::E01006);

    /// 01004
    pub const WARNING_STRING_DATA_RIGHT_TRUNCATION: SqlState = SqlState(Inner::E01004);

    /// 01P01
    pub const WARNING_DEPRECATED_FEATURE: SqlState = SqlState(Inner::E01P01);

    /// 02000
    pub const NO_DATA: SqlState = SqlState(Inner::E02000);

    /// 02001
    pub const NO_ADDITIONAL_DYNAMIC_RESULT_SETS_RETURNED: SqlState = SqlState(Inner::E02001);

    /// 03000
    pub const SQL_STATEMENT_NOT_YET_COMPLETE: SqlState = SqlState(Inner::E03000);

    /// 08000
    pub const CONNECTION_EXCEPTION: SqlState = SqlState(Inner::E08000);

    /// 08003
    pub const CONNECTION_DOES_NOT_EXIST: SqlState = SqlState(Inner::E08003);

    /// 08006
    pub const CONNECTION_FAILURE: SqlState = SqlState(Inner::E08006);

    /// 08001
    pub const SQLCLIENT_UNABLE_TO_ESTABLISH_SQLCONNECTION: SqlState = SqlState(Inner::E08001);

    /// 08004
    pub const SQLSERVER_REJECTED_ESTABLISHMENT_OF_SQLCONNECTION: SqlState = SqlState(Inner::E08004);

    /// 08007
    pub const TRANSACTION_RESOLUTION_UNKNOWN: SqlState = SqlState(Inner::E08007);

    /// 08P01
    pub const PROTOCOL_VIOLATION: SqlState = SqlState(Inner::E08P01);

    /// 09000
    pub const TRIGGERED_ACTION_EXCEPTION: SqlState = SqlState(Inner::E09000);

    /// 0A000
    pub const FEATURE_NOT_SUPPORTED: SqlState = SqlState(Inner::E0A000);

    /// 0B000
    pub const INVALID_TRANSACTION_INITIATION: SqlState = SqlState(Inner::E0B000);

    /// 0F000
    pub const LOCATOR_EXCEPTION: SqlState = SqlState(Inner::E0F000);

    /// 0F001
    pub const L_E_INVALID_SPECIFICATION: SqlState = SqlState(Inner::E0F001);

    /// 0L000
    pub const INVALID_GRANTOR: SqlState = SqlState(Inner::E0L000);

    /// 0LP01
    pub const INVALID_GRANT_OPERATION: SqlState = SqlState(Inner::E0LP01);

    /// 0P000
    pub const INVALID_ROLE_SPECIFICATION: SqlState = SqlState(Inner::E0P000);

    /// 0Z000
    pub const DIAGNOSTICS_EXCEPTION: SqlState = SqlState(Inner::E0Z000);

    /// 0Z002
    pub const STACKED_DIAGNOSTICS_ACCESSED_WITHOUT_ACTIVE_HANDLER: SqlState = SqlState(Inner::E0Z002);

    /// 20000
    pub const CASE_NOT_FOUND: SqlState = SqlState(Inner::E20000);

    /// 21000
    pub const CARDINALITY_VIOLATION: SqlState = SqlState(Inner::E21000);

    /// 22000
    pub const DATA_EXCEPTION: SqlState = SqlState(Inner::E22000);

    /// 2202E
    pub const ARRAY_ELEMENT_ERROR: SqlState = SqlState(Inner::E2202E);

    /// 2202E
    pub const ARRAY_SUBSCRIPT_ERROR: SqlState = SqlState(Inner::E2202E);

    /// 22021
    pub const CHARACTER_NOT_IN_REPERTOIRE: SqlState = SqlState(Inner::E22021);

    /// 22008
    pub const DATETIME_FIELD_OVERFLOW: SqlState = SqlState(Inner::E22008);

    /// 22008
    pub const DATETIME_VALUE_OUT_OF_RANGE: SqlState = SqlState(Inner::E22008);

    /// 22012
    pub const DIVISION_BY_ZERO: SqlState = SqlState(Inner::E22012);

    /// 22005
    pub const ERROR_IN_ASSIGNMENT: SqlState = SqlState(Inner::E22005);

    /// 2200B
    pub const ESCAPE_CHARACTER_CONFLICT: SqlState = SqlState(Inner::E2200B);

    /// 22022
    pub const INDICATOR_OVERFLOW: SqlState = SqlState(Inner::E22022);

    /// 22015
    pub const INTERVAL_FIELD_OVERFLOW: SqlState = SqlState(Inner::E22015);

    /// 2201E
    pub const INVALID_ARGUMENT_FOR_LOG: SqlState = SqlState(Inner::E2201E);

    /// 22014
    pub const INVALID_ARGUMENT_FOR_NTILE: SqlState = SqlState(Inner::E22014);

    /// 22016
    pub const INVALID_ARGUMENT_FOR_NTH_VALUE: SqlState = SqlState(Inner::E22016);

    /// 2201F
    pub const INVALID_ARGUMENT_FOR_POWER_FUNCTION: SqlState = SqlState(Inner::E2201F);

    /// 2201G
    pub const INVALID_ARGUMENT_FOR_WIDTH_BUCKET_FUNCTION: SqlState = SqlState(Inner::E2201G);

    /// 22018
    pub const INVALID_CHARACTER_VALUE_FOR_CAST: SqlState = SqlState(Inner::E22018);

    /// 22007
    pub const INVALID_DATETIME_FORMAT: SqlState = SqlState(Inner::E22007);

    /// 22019
    pub const INVALID_ESCAPE_CHARACTER: SqlState = SqlState(Inner::E22019);

    /// 2200D
    pub const INVALID_ESCAPE_OCTET: SqlState = SqlState(Inner::E2200D);

    /// 22025
    pub const INVALID_ESCAPE_SEQUENCE: SqlState = SqlState(Inner::E22025);

    /// 22P06
    pub const NONSTANDARD_USE_OF_ESCAPE_CHARACTER: SqlState = SqlState(Inner::E22P06);

    /// 22010
    pub const INVALID_INDICATOR_PARAMETER_VALUE: SqlState = SqlState(Inner::E22010);

    /// 22023
    pub const INVALID_PARAMETER_VALUE: SqlState = SqlState(Inner::E22023);

    /// 22013
    pub const INVALID_PRECEDING_OR_FOLLOWING_SIZE: SqlState = SqlState(Inner::E22013);

    /// 2201B
    pub const INVALID_REGULAR_EXPRESSION: SqlState = SqlState(Inner::E2201B);

    /// 2201W
    pub const INVALID_ROW_COUNT_IN_LIMIT_CLAUSE: SqlState = SqlState(Inner::E2201W);

    /// 2201X
    pub const INVALID_ROW_COUNT_IN_RESULT_OFFSET_CLAUSE: SqlState = SqlState(Inner::E2201X);

    /// 2202H
    pub const INVALID_TABLESAMPLE_ARGUMENT: SqlState = SqlState(Inner::E2202H);

    /// 2202G
    pub const INVALID_TABLESAMPLE_REPEAT: SqlState = SqlState(Inner::E2202G);

    /// 22009
    pub const INVALID_TIME_ZONE_DISPLACEMENT_VALUE: SqlState = SqlState(Inner::E22009);

    /// 2200C
    pub const INVALID_USE_OF_ESCAPE_CHARACTER: SqlState = SqlState(Inner::E2200C);

    /// 2200G
    pub const MOST_SPECIFIC_TYPE_MISMATCH: SqlState = SqlState(Inner::E2200G);

    /// 22004
    pub const NULL_VALUE_NOT_ALLOWED: SqlState = SqlState(Inner::E22004);

    /// 22002
    pub const NULL_VALUE_NO_INDICATOR_PARAMETER: SqlState = SqlState(Inner::E22002);

    /// 22003
    pub const NUMERIC_VALUE_OUT_OF_RANGE: SqlState = SqlState(Inner::E22003);

    /// 2200H
    pub const SEQUENCE_GENERATOR_LIMIT_EXCEEDED: SqlState = SqlState(Inner::E2200H);

    /// 22026
    pub const STRING_DATA_LENGTH_MISMATCH: SqlState = SqlState(Inner::E22026);

    /// 22001
    pub const STRING_DATA_RIGHT_TRUNCATION: SqlState = SqlState(Inner::E22001);

    /// 22011
    pub const SUBSTRING_ERROR: SqlState = SqlState(Inner::E22011);

    /// 22027
    pub const TRIM_ERROR: SqlState = SqlState(Inner::E22027);

    /// 22024
    pub const UNTERMINATED_C_STRING: SqlState = SqlState(Inner::E22024);

    /// 2200F
    pub const ZERO_LENGTH_CHARACTER_STRING: SqlState = SqlState(Inner::E2200F);

    /// 22P01
    pub const FLOATING_POINT_EXCEPTION: SqlState = SqlState(Inner::E22P01);

    /// 22P02
    pub const INVALID_TEXT_REPRESENTATION: SqlState = SqlState(Inner::E22P02);

    /// 22P03
    pub const INVALID_BINARY_REPRESENTATION: SqlState = SqlState(Inner::E22P03);

    /// 22P04
    pub const BAD_COPY_FILE_FORMAT: SqlState = SqlState(Inner::E22P04);

    /// 22P05
    pub const UNTRANSLATABLE_CHARACTER: SqlState = SqlState(Inner::E22P05);

    /// 2200L
    pub const NOT_AN_XML_DOCUMENT: SqlState = SqlState(Inner::E2200L);

    /// 2200M
    pub const INVALID_XML_DOCUMENT: SqlState = SqlState(Inner::E2200M);

    /// 2200N
    pub const INVALID_XML_CONTENT: SqlState = SqlState(Inner::E2200N);

    /// 2200S
    pub const INVALID_XML_COMMENT: SqlState = SqlState(Inner::E2200S);

    /// 2200T
    pub const INVALID_XML_PROCESSING_INSTRUCTION: SqlState = SqlState(Inner::E2200T);

    /// 22030
    pub const DUPLICATE_JSON_OBJECT_KEY_VALUE: SqlState = SqlState(Inner::E22030);

    /// 22031
    pub const INVALID_ARGUMENT_FOR_SQL_JSON_DATETIME_FUNCTION: SqlState = SqlState(Inner::E22031);

    /// 22032
    pub const INVALID_JSON_TEXT: SqlState = SqlState(Inner::E22032);

    /// 22033
    pub const INVALID_SQL_JSON_SUBSCRIPT: SqlState = SqlState(Inner::E22033);

    /// 22034
    pub const MORE_THAN_ONE_SQL_JSON_ITEM: SqlState = SqlState(Inner::E22034);

    /// 22035
    pub const NO_SQL_JSON_ITEM: SqlState = SqlState(Inner::E22035);

    /// 22036
    pub const NON_NUMERIC_SQL_JSON_ITEM: SqlState = SqlState(Inner::E22036);

    /// 22037
    pub const NON_UNIQUE_KEYS_IN_A_JSON_OBJECT: SqlState = SqlState(Inner::E22037);

    /// 22038
    pub const SINGLETON_SQL_JSON_ITEM_REQUIRED: SqlState = SqlState(Inner::E22038);

    /// 22039
    pub const SQL_JSON_ARRAY_NOT_FOUND: SqlState = SqlState(Inner::E22039);

    /// 2203A
    pub const SQL_JSON_MEMBER_NOT_FOUND: SqlState = SqlState(Inner::E2203A);

    /// 2203B
    pub const SQL_JSON_NUMBER_NOT_FOUND: SqlState = SqlState(Inner::E2203B);

    /// 2203C
    pub const SQL_JSON_OBJECT_NOT_FOUND: SqlState = SqlState(Inner::E2203C);

    /// 2203D
    pub const TOO_MANY_JSON_ARRAY_ELEMENTS: SqlState = SqlState(Inner::E2203D);

    /// 2203E
    pub const TOO_MANY_JSON_OBJECT_MEMBERS: SqlState = SqlState(Inner::E2203E);

    /// 2203F
    pub const SQL_JSON_SCALAR_REQUIRED: SqlState = SqlState(Inner::E2203F);

    /// 2203G
    pub const SQL_JSON_ITEM_CANNOT_BE_CAST_TO_TARGET_TYPE: SqlState = SqlState(Inner::E2203G);

    /// 23000
    pub const INTEGRITY_CONSTRAINT_VIOLATION: SqlState = SqlState(Inner::E23000);

    /// 23001
    pub const RESTRICT_VIOLATION: SqlState = SqlState(Inner::E23001);

    /// 23502
    pub const NOT_NULL_VIOLATION: SqlState = SqlState(Inner::E23502);

    /// 23503
    pub const FOREIGN_KEY_VIOLATION: SqlState = SqlState(Inner::E23503);

    /// 23505
    pub const UNIQUE_VIOLATION: SqlState = SqlState(Inner::E23505);

    /// 23514
    pub const CHECK_VIOLATION: SqlState = SqlState(Inner::E23514);

    /// 23P01
    pub const EXCLUSION_VIOLATION: SqlState = SqlState(Inner::E23P01);

    /// 24000
    pub const INVALID_CURSOR_STATE: SqlState = SqlState(Inner::E24000);

    /// 25000
    pub const INVALID_TRANSACTION_STATE: SqlState = SqlState(Inner::E25000);

    /// 25001
    pub const ACTIVE_SQL_TRANSACTION: SqlState = SqlState(Inner::E25001);

    /// 25002
    pub const BRANCH_TRANSACTION_ALREADY_ACTIVE: SqlState = SqlState(Inner::E25002);

    /// 25008
    pub const HELD_CURSOR_REQUIRES_SAME_ISOLATION_LEVEL: SqlState = SqlState(Inner::E25008);

    /// 25003
    pub const INAPPROPRIATE_ACCESS_MODE_FOR_BRANCH_TRANSACTION: SqlState = SqlState(Inner::E25003);

    /// 25004
    pub const INAPPROPRIATE_ISOLATION_LEVEL_FOR_BRANCH_TRANSACTION: SqlState = SqlState(Inner::E25004);

    /// 25005
    pub const NO_ACTIVE_SQL_TRANSACTION_FOR_BRANCH_TRANSACTION: SqlState = SqlState(Inner::E25005);

    /// 25006
    pub const READ_ONLY_SQL_TRANSACTION: SqlState = SqlState(Inner::E25006);

    /// 25007
    pub const SCHEMA_AND_DATA_STATEMENT_MIXING_NOT_SUPPORTED: SqlState = SqlState(Inner::E25007);

    /// 25P01
    pub const NO_ACTIVE_SQL_TRANSACTION: SqlState = SqlState(Inner::E25P01);

    /// 25P02
    pub const IN_FAILED_SQL_TRANSACTION: SqlState = SqlState(Inner::E25P02);

    /// 25P03
    pub const IDLE_IN_TRANSACTION_SESSION_TIMEOUT: SqlState = SqlState(Inner::E25P03);

    /// 26000
    pub const INVALID_SQL_STATEMENT_NAME: SqlState = SqlState(Inner::E26000);

    /// 26000
    pub const UNDEFINED_PSTATEMENT: SqlState = SqlState(Inner::E26000);

    /// 27000
    pub const TRIGGERED_DATA_CHANGE_VIOLATION: SqlState = SqlState(Inner::E27000);

    /// 28000
    pub const INVALID_AUTHORIZATION_SPECIFICATION: SqlState = SqlState(Inner::E28000);

    /// 28P01
    pub const INVALID_PASSWORD: SqlState = SqlState(Inner::E28P01);

    /// 2B000
    pub const DEPENDENT_PRIVILEGE_DESCRIPTORS_STILL_EXIST: SqlState = SqlState(Inner::E2B000);

    /// 2BP01
    pub const DEPENDENT_OBJECTS_STILL_EXIST: SqlState = SqlState(Inner::E2BP01);

    /// 2D000
    pub const INVALID_TRANSACTION_TERMINATION: SqlState = SqlState(Inner::E2D000);

    /// 2F000
    pub const SQL_ROUTINE_EXCEPTION: SqlState = SqlState(Inner::E2F000);

    /// 2F005
    pub const S_R_E_FUNCTION_EXECUTED_NO_RETURN_STATEMENT: SqlState = SqlState(Inner::E2F005);

    /// 2F002
    pub const S_R_E_MODIFYING_SQL_DATA_NOT_PERMITTED: SqlState = SqlState(Inner::E2F002);

    /// 2F003
    pub const S_R_E_PROHIBITED_SQL_STATEMENT_ATTEMPTED: SqlState = SqlState(Inner::E2F003);

    /// 2F004
    pub const S_R_E_READING_SQL_DATA_NOT_PERMITTED: SqlState = SqlState(Inner::E2F004);

    /// 34000
    pub const INVALID_CURSOR_NAME: SqlState = SqlState(Inner::E34000);

    /// 34000
    pub const UNDEFINED_CURSOR: SqlState = SqlState(Inner::E34000);

    /// 38000
    pub const EXTERNAL_ROUTINE_EXCEPTION: SqlState = SqlState(Inner::E38000);

    /// 38001
    pub const E_R_E_CONTAINING_SQL_NOT_PERMITTED: SqlState = SqlState(Inner::E38001);

    /// 38002
    pub const E_R_E_MODIFYING_SQL_DATA_NOT_PERMITTED: SqlState = SqlState(Inner::E38002);

    /// 38003
    pub const E_R_E_PROHIBITED_SQL_STATEMENT_ATTEMPTED: SqlState = SqlState(Inner::E38003);

    /// 38004
    pub const E_R_E_READING_SQL_DATA_NOT_PERMITTED: SqlState = SqlState(Inner::E38004);

    /// 39000
    pub const EXTERNAL_ROUTINE_INVOCATION_EXCEPTION: SqlState = SqlState(Inner::E39000);

    /// 39001
    pub const E_R_I_E_INVALID_SQLSTATE_RETURNED: SqlState = SqlState(Inner::E39001);

    /// 39004
    pub const E_R_I_E_NULL_VALUE_NOT_ALLOWED: SqlState = SqlState(Inner::E39004);

    /// 39P01
    pub const E_R_I_E_TRIGGER_PROTOCOL_VIOLATED: SqlState = SqlState(Inner::E39P01);

    /// 39P02
    pub const E_R_I_E_SRF_PROTOCOL_VIOLATED: SqlState = SqlState(Inner::E39P02);

    /// 39P03
    pub const E_R_I_E_EVENT_TRIGGER_PROTOCOL_VIOLATED: SqlState = SqlState(Inner::E39P03);

    /// 3B000
    pub const SAVEPOINT_EXCEPTION: SqlState = SqlState(Inner::E3B000);

    /// 3B001
    pub const S_E_INVALID_SPECIFICATION: SqlState = SqlState(Inner::E3B001);

    /// 3D000
    pub const INVALID_CATALOG_NAME: SqlState = SqlState(Inner::E3D000);

    /// 3D000
    pub const UNDEFINED_DATABASE: SqlState = SqlState(Inner::E3D000);

    /// 3F000
    pub const INVALID_SCHEMA_NAME: SqlState = SqlState(Inner::E3F000);

    /// 3F000
    pub const UNDEFINED_SCHEMA: SqlState = SqlState(Inner::E3F000);

    /// 40000
    pub const TRANSACTION_ROLLBACK: SqlState = SqlState(Inner::E40000);

    /// 40002
    pub const T_R_INTEGRITY_CONSTRAINT_VIOLATION: SqlState = SqlState(Inner::E40002);

    /// 40001
    pub const T_R_SERIALIZATION_FAILURE: SqlState = SqlState(Inner::E40001);

    /// 40003
    pub const T_R_STATEMENT_COMPLETION_UNKNOWN: SqlState = SqlState(Inner::E40003);

    /// 40P01
    pub const T_R_DEADLOCK_DETECTED: SqlState = SqlState(Inner::E40P01);

    /// 42000
    pub const SYNTAX_ERROR_OR_ACCESS_RULE_VIOLATION: SqlState = SqlState(Inner::E42000);

    /// 42601
    pub const SYNTAX_ERROR: SqlState = SqlState(Inner::E42601);

    /// 42501
    pub const INSUFFICIENT_PRIVILEGE: SqlState = SqlState(Inner::E42501);

    /// 42846
    pub const CANNOT_COERCE: SqlState = SqlState(Inner::E42846);

    /// 42803
    pub const GROUPING_ERROR: SqlState = SqlState(Inner::E42803);

    /// 42P20
    pub const WINDOWING_ERROR: SqlState = SqlState(Inner::E42P20);

    /// 42P19
    pub const INVALID_RECURSION: SqlState = SqlState(Inner::E42P19);

    /// 42830
    pub const INVALID_FOREIGN_KEY: SqlState = SqlState(Inner::E42830);

    /// 42602
    pub const INVALID_NAME: SqlState = SqlState(Inner::E42602);

    /// 42622
    pub const NAME_TOO_LONG: SqlState = SqlState(Inner::E42622);

    /// 42939
    pub const RESERVED_NAME: SqlState = SqlState(Inner::E42939);

    /// 42804
    pub const DATATYPE_MISMATCH: SqlState = SqlState(Inner::E42804);

    /// 42P18
    pub const INDETERMINATE_DATATYPE: SqlState = SqlState(Inner::E42P18);

    /// 42P21
    pub const COLLATION_MISMATCH: SqlState = SqlState(Inner::E42P21);

    /// 42P22
    pub const INDETERMINATE_COLLATION: SqlState = SqlState(Inner::E42P22);

    /// 42809
    pub const WRONG_OBJECT_TYPE: SqlState = SqlState(Inner::E42809);

    /// 428C9
    pub const GENERATED_ALWAYS: SqlState = SqlState(Inner::E428C9);

    /// 42703
    pub const UNDEFINED_COLUMN: SqlState = SqlState(Inner::E42703);

    /// 42883
    pub const UNDEFINED_FUNCTION: SqlState = SqlState(Inner::E42883);

    /// 42P01
    pub const UNDEFINED_TABLE: SqlState = SqlState(Inner::E42P01);

    /// 42P02
    pub const UNDEFINED_PARAMETER: SqlState = SqlState(Inner::E42P02);

    /// 42704
    pub const UNDEFINED_OBJECT: SqlState = SqlState(Inner::E42704);

    /// 42701
    pub const DUPLICATE_COLUMN: SqlState = SqlState(Inner::E42701);

    /// 42P03
    pub const DUPLICATE_CURSOR: SqlState = SqlState(Inner::E42P03);

    /// 42P04
    pub const DUPLICATE_DATABASE: SqlState = SqlState(Inner::E42P04);

    /// 42723
    pub const DUPLICATE_FUNCTION: SqlState = SqlState(Inner::E42723);

    /// 42P05
    pub const DUPLICATE_PSTATEMENT: SqlState = SqlState(Inner::E42P05);

    /// 42P06
    pub const DUPLICATE_SCHEMA: SqlState = SqlState(Inner::E42P06);

    /// 42P07
    pub const DUPLICATE_TABLE: SqlState = SqlState(Inner::E42P07);

    /// 42712
    pub const DUPLICATE_ALIAS: SqlState = SqlState(Inner::E42712);

    /// 42710
    pub const DUPLICATE_OBJECT: SqlState = SqlState(Inner::E42710);

    /// 42702
    pub const AMBIGUOUS_COLUMN: SqlState = SqlState(Inner::E42702);

    /// 42725
    pub const AMBIGUOUS_FUNCTION: SqlState = SqlState(Inner::E42725);

    /// 42P08
    pub const AMBIGUOUS_PARAMETER: SqlState = SqlState(Inner::E42P08);

    /// 42P09
    pub const AMBIGUOUS_ALIAS: SqlState = SqlState(Inner::E42P09);

    /// 42P10
    pub const INVALID_COLUMN_REFERENCE: SqlState = SqlState(Inner::E42P10);

    /// 42611
    pub const INVALID_COLUMN_DEFINITION: SqlState = SqlState(Inner::E42611);

    /// 42P11
    pub const INVALID_CURSOR_DEFINITION: SqlState = SqlState(Inner::E42P11);

    /// 42P12
    pub const INVALID_DATABASE_DEFINITION: SqlState = SqlState(Inner::E42P12);

    /// 42P13
    pub const INVALID_FUNCTION_DEFINITION: SqlState = SqlState(Inner::E42P13);

    /// 42P14
    pub const INVALID_PSTATEMENT_DEFINITION: SqlState = SqlState(Inner::E42P14);

    /// 42P15
    pub const INVALID_SCHEMA_DEFINITION: SqlState = SqlState(Inner::E42P15);

    /// 42P16
    pub const INVALID_TABLE_DEFINITION: SqlState = SqlState(Inner::E42P16);

    /// 42P17
    pub const INVALID_OBJECT_DEFINITION: SqlState = SqlState(Inner::E42P17);

    /// 44000
    pub const WITH_CHECK_OPTION_VIOLATION: SqlState = SqlState(Inner::E44000);

    /// 53000
    pub const INSUFFICIENT_RESOURCES: SqlState = SqlState(Inner::E53000);

    /// 53100
    pub const DISK_FULL: SqlState = SqlState(Inner::E53100);

    /// 53200
    pub const OUT_OF_MEMORY: SqlState = SqlState(Inner::E53200);

    /// 53300
    pub const TOO_MANY_CONNECTIONS: SqlState = SqlState(Inner::E53300);

    /// 53400
    pub const CONFIGURATION_LIMIT_EXCEEDED: SqlState = SqlState(Inner::E53400);

    /// 54000
    pub const PROGRAM_LIMIT_EXCEEDED: SqlState = SqlState(Inner::E54000);

    /// 54001
    pub const STATEMENT_TOO_COMPLEX: SqlState = SqlState(Inner::E54001);

    /// 54011
    pub const TOO_MANY_COLUMNS: SqlState = SqlState(Inner::E54011);

    /// 54023
    pub const TOO_MANY_ARGUMENTS: SqlState = SqlState(Inner::E54023);

    /// 55000
    pub const OBJECT_NOT_IN_PREREQUISITE_STATE: SqlState = SqlState(Inner::E55000);

    /// 55006
    pub const OBJECT_IN_USE: SqlState = SqlState(Inner::E55006);

    /// 55P02
    pub const CANT_CHANGE_RUNTIME_PARAM: SqlState = SqlState(Inner::E55P02);

    /// 55P03
    pub const LOCK_NOT_AVAILABLE: SqlState = SqlState(Inner::E55P03);

    /// 55P04
    pub const UNSAFE_NEW_ENUM_VALUE_USAGE: SqlState = SqlState(Inner::E55P04);

    /// 57000
    pub const OPERATOR_INTERVENTION: SqlState = SqlState(Inner::E57000);

    /// 57014
    pub const QUERY_CANCELED: SqlState = SqlState(Inner::E57014);

    /// 57P01
    pub const ADMIN_SHUTDOWN: SqlState = SqlState(Inner::E57P01);

    /// 57P02
    pub const CRASH_SHUTDOWN: SqlState = SqlState(Inner::E57P02);

    /// 57P03
    pub const CANNOT_CONNECT_NOW: SqlState = SqlState(Inner::E57P03);

    /// 57P04
    pub const DATABASE_DROPPED: SqlState = SqlState(Inner::E57P04);

    /// 57P05
    pub const IDLE_SESSION_TIMEOUT: SqlState = SqlState(Inner::E57P05);

    /// 58000
    pub const SYSTEM_ERROR: SqlState = SqlState(Inner::E58000);

    /// 58030
    pub const IO_ERROR: SqlState = SqlState(Inner::E58030);

    /// 58P01
    pub const UNDEFINED_FILE: SqlState = SqlState(Inner::E58P01);

    /// 58P02
    pub const DUPLICATE_FILE: SqlState = SqlState(Inner::E58P02);

    /// 72000
    pub const SNAPSHOT_TOO_OLD: SqlState = SqlState(Inner::E72000);

    /// F0000
    pub const CONFIG_FILE_ERROR: SqlState = SqlState(Inner::EF0000);

    /// F0001
    pub const LOCK_FILE_EXISTS: SqlState = SqlState(Inner::EF0001);

    /// HV000
    pub const FDW_ERROR: SqlState = SqlState(Inner::EHV000);

    /// HV005
    pub const FDW_COLUMN_NAME_NOT_FOUND: SqlState = SqlState(Inner::EHV005);

    /// HV002
    pub const FDW_DYNAMIC_PARAMETER_VALUE_NEEDED: SqlState = SqlState(Inner::EHV002);

    /// HV010
    pub const FDW_FUNCTION_SEQUENCE_ERROR: SqlState = SqlState(Inner::EHV010);

    /// HV021
    pub const FDW_INCONSISTENT_DESCRIPTOR_INFORMATION: SqlState = SqlState(Inner::EHV021);

    /// HV024
    pub const FDW_INVALID_ATTRIBUTE_VALUE: SqlState = SqlState(Inner::EHV024);

    /// HV007
    pub const FDW_INVALID_COLUMN_NAME: SqlState = SqlState(Inner::EHV007);

    /// HV008
    pub const FDW_INVALID_COLUMN_NUMBER: SqlState = SqlState(Inner::EHV008);

    /// HV004
    pub const FDW_INVALID_DATA_TYPE: SqlState = SqlState(Inner::EHV004);

    /// HV006
    pub const FDW_INVALID_DATA_TYPE_DESCRIPTORS: SqlState = SqlState(Inner::EHV006);

    /// HV091
    pub const FDW_INVALID_DESCRIPTOR_FIELD_IDENTIFIER: SqlState = SqlState(Inner::EHV091);

    /// HV00B
    pub const FDW_INVALID_HANDLE: SqlState = SqlState(Inner::EHV00B);

    /// HV00C
    pub const FDW_INVALID_OPTION_INDEX: SqlState = SqlState(Inner::EHV00C);

    /// HV00D
    pub const FDW_INVALID_OPTION_NAME: SqlState = SqlState(Inner::EHV00D);

    /// HV090
    pub const FDW_INVALID_STRING_LENGTH_OR_BUFFER_LENGTH: SqlState = SqlState(Inner::EHV090);

    /// HV00A
    pub const FDW_INVALID_STRING_FORMAT: SqlState = SqlState(Inner::EHV00A);

    /// HV009
    pub const FDW_INVALID_USE_OF_NULL_POINTER: SqlState = SqlState(Inner::EHV009);

    /// HV014
    pub const FDW_TOO_MANY_HANDLES: SqlState = SqlState(Inner::EHV014);

    /// HV001
    pub const FDW_OUT_OF_MEMORY: SqlState = SqlState(Inner::EHV001);

    /// HV00P
    pub const FDW_NO_SCHEMAS: SqlState = SqlState(Inner::EHV00P);

    /// HV00J
    pub const FDW_OPTION_NAME_NOT_FOUND: SqlState = SqlState(Inner::EHV00J);

    /// HV00K
    pub const FDW_REPLY_HANDLE: SqlState = SqlState(Inner::EHV00K);

    /// HV00Q
    pub const FDW_SCHEMA_NOT_FOUND: SqlState = SqlState(Inner::EHV00Q);

    /// HV00R
    pub const FDW_TABLE_NOT_FOUND: SqlState = SqlState(Inner::EHV00R);

    /// HV00L
    pub const FDW_UNABLE_TO_CREATE_EXECUTION: SqlState = SqlState(Inner::EHV00L);

    /// HV00M
    pub const FDW_UNABLE_TO_CREATE_REPLY: SqlState = SqlState(Inner::EHV00M);

    /// HV00N
    pub const FDW_UNABLE_TO_ESTABLISH_CONNECTION: SqlState = SqlState(Inner::EHV00N);

    /// P0000
    pub const PLPGSQL_ERROR: SqlState = SqlState(Inner::EP0000);

    /// P0001
    pub const RAISE_EXCEPTION: SqlState = SqlState(Inner::EP0001);

    /// P0002
    pub const NO_DATA_FOUND: SqlState = SqlState(Inner::EP0002);

    /// P0003
    pub const TOO_MANY_ROWS: SqlState = SqlState(Inner::EP0003);

    /// P0004
    pub const ASSERT_FAILURE: SqlState = SqlState(Inner::EP0004);

    /// XX000
    pub const INTERNAL_ERROR: SqlState = SqlState(Inner::EXX000);

    /// XX001
    pub const DATA_CORRUPTED: SqlState = SqlState(Inner::EXX001);

    /// XX002
    pub const INDEX_CORRUPTED: SqlState = SqlState(Inner::EXX002);
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[allow(clippy::upper_case_acronyms)]
enum Inner {
    E00000,
    E01000,
    E0100C,
    E01008,
    E01003,
    E01007,
    E01006,
    E01004,
    E01P01,
    E02000,
    E02001,
    E03000,
    E08000,
    E08003,
    E08006,
    E08001,
    E08004,
    E08007,
    E08P01,
    E09000,
    E0A000,
    E0B000,
    E0F000,
    E0F001,
    E0L000,
    E0LP01,
    E0P000,
    E0Z000,
    E0Z002,
    E20000,
    E21000,
    E22000,
    E2202E,
    E22021,
    E22008,
    E22012,
    E22005,
    E2200B,
    E22022,
    E22015,
    E2201E,
    E22014,
    E22016,
    E2201F,
    E2201G,
    E22018,
    E22007,
    E22019,
    E2200D,
    E22025,
    E22P06,
    E22010,
    E22023,
    E22013,
    E2201B,
    E2201W,
    E2201X,
    E2202H,
    E2202G,
    E22009,
    E2200C,
    E2200G,
    E22004,
    E22002,
    E22003,
    E2200H,
    E22026,
    E22001,
    E22011,
    E22027,
    E22024,
    E2200F,
    E22P01,
    E22P02,
    E22P03,
    E22P04,
    E22P05,
    E2200L,
    E2200M,
    E2200N,
    E2200S,
    E2200T,
    E22030,
    E22031,
    E22032,
    E22033,
    E22034,
    E22035,
    E22036,
    E22037,
    E22038,
    E22039,
    E2203A,
    E2203B,
    E2203C,
    E2203D,
    E2203E,
    E2203F,
    E2203G,
    E23000,
    E23001,
    E23502,
    E23503,
    E23505,
    E23514,
    E23P01,
    E24000,
    E25000,
    E25001,
    E25002,
    E25008,
    E25003,
    E25004,
    E25005,
    E25006,
    E25007,
    E25P01,
    E25P02,
    E25P03,
    E26000,
    E27000,
    E28000,
    E28P01,
    E2B000,
    E2BP01,
    E2D000,
    E2F000,
    E2F005,
    E2F002,
    E2F003,
    E2F004,
    E34000,
    E38000,
    E38001,
    E38002,
    E38003,
    E38004,
    E39000,
    E39001,
    E39004,
    E39P01,
    E39P02,
    E39P03,
    E3B000,
    E3B001,
    E3D000,
    E3F000,
    E40000,
    E40002,
    E40001,
    E40003,
    E40P01,
    E42000,
    E42601,
    E42501,
    E42846,
    E42803,
    E42P20,
    E42P19,
    E42830,
    E42602,
    E42622,
    E42939,
    E42804,
    E42P18,
    E42P21,
    E42P22,
    E42809,
    E428C9,
    E42703,
    E42883,
    E42P01,
    E42P02,
    E42704,
    E42701,
    E42P03,
    E42P04,
    E42723,
    E42P05,
    E42P06,
    E42P07,
    E42712,
    E42710,
    E42702,
    E42725,
    E42P08,
    E42P09,
    E42P10,
    E42611,
    E42P11,
    E42P12,
    E42P13,
    E42P14,
    E42P15,
    E42P16,
    E42P17,
    E44000,
    E53000,
    E53100,
    E53200,
    E53300,
    E53400,
    E54000,
    E54001,
    E54011,
    E54023,
    E55000,
    E55006,
    E55P02,
    E55P03,
    E55P04,
    E57000,
    E57014,
    E57P01,
    E57P02,
    E57P03,
    E57P04,
    E57P05,
    E58000,
    E58030,
    E58P01,
    E58P02,
    E72000,
    EF0000,
    EF0001,
    EHV000,
    EHV005,
    EHV002,
    EHV010,
    EHV021,
    EHV024,
    EHV007,
    EHV008,
    EHV004,
    EHV006,
    EHV091,
    EHV00B,
    EHV00C,
    EHV00D,
    EHV090,
    EHV00A,
    EHV009,
    EHV014,
    EHV001,
    EHV00P,
    EHV00J,
    EHV00K,
    EHV00Q,
    EHV00R,
    EHV00L,
    EHV00M,
    EHV00N,
    EP0000,
    EP0001,
    EP0002,
    EP0003,
    EP0004,
    EXX000,
    EXX001,
    EXX002,
    Other(Box<str>),
}
