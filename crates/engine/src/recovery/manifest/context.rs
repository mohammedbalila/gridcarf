//! One finite classification for lexical, duplicate-visitor and serializer-observer accounting.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    General,
    Metadata,
    Xml,
    Base64,
    Root,
    Workbook,
    Sheet,
    MetadataRecord,
    Sidecars,
    WorkbookSidecar,
    SheetSidecar,
    Preserved,
    Part,
    OptionalPart,
    Relationship,
    Issue,
    Fragment,
    KeptSheet,
    NativePath,
    U8,
    U16,
    Group,
    CellStore,
    Bands,
    Band,
    StoredRows,
    StoredCols,
    Lines,
    Cell,
    Value,
    ValuePayload,
    Python,
    PythonOutput,
    Comments,
    Hyperlinks,
    Spill,
    SpillRanges,
    Comment,
    Hyperlink,
    Image,
    PicturePair,
    DefinedName,
    DocProps,
    StyleTable,
    Style,
    Theme,
    Font,
    Dxf,
    NamedStylePair,
    ScriptPair,
    ReplyPair,
    Phonetic,
    PhoneticProps,
    Table,
    TableColumn,
    XmlColumn,
    AutoFilter,
    FilterPair,
    Filter,
    FilterIcon,
    CondFormat,
    CfRule,
    CfStyle,
    CfTime,
    CfIcon,
    Chart,
    Shape,
    ShapeFormat,
    Control,
    Action,
    Slicer,
    Pivot,
    PivotField,
    PivotCalc,
    Print,
    Protection,
    EditRange,
    Password,
    IteratedPassword,
    Macro,
    NamedInput,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Context {
    One(Kind),
    List(Kind),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyKind {
    Decimal,
    A1,
}

impl Context {
    pub(super) const ROOT: Self = Self::One(Kind::Root);
    pub(super) const GENERAL: Self = Self::One(Kind::General);

    pub(super) fn field(self, key: &str) -> Self {
        use Context::{List, One};
        use Kind::*;
        match (self, key) {
            (One(Bands), _) => One(Band),
            (One(StoredRows), _) => One(StoredCols),
            (One(StoredCols), _) => One(Cell),
            (One(Lines | SpillRanges), _) => Self::GENERAL,
            (One(Comments), _) => One(Comment),
            (One(Hyperlinks), _) => One(Hyperlink),
            (One(Spill), _) => One(Value),
            (One(Root), "workbook") => One(Workbook),
            (One(Root), "metadata") => One(MetadataRecord),
            (One(Root), "sidecars") => One(Sidecars),
            (One(Workbook), "sheets") => List(Sheet),
            (One(Workbook), "names") => List(DefinedName),
            (One(Workbook), "styles") => One(StyleTable),
            (One(Workbook), "theme") => One(Theme),
            (One(Workbook), "props") => One(DocProps),
            (One(Workbook), "cell_styles") => List(NamedStylePair),
            (One(Workbook), "phonetics") => List(Phonetic),
            (One(Workbook), "vba_project") => One(Base64),
            (One(Workbook), "macros") => List(Macro),
            (One(Workbook), "structure_password") => One(Password),
            (One(Sheet), "cells") => One(CellStore),
            (One(Sheet), "cols" | "rows") => One(Lines),
            (One(Sheet), "comments") => One(Comments),
            (One(Sheet), "hyperlinks") => One(Hyperlinks),
            (One(Sheet), "cell_pictures") => List(PicturePair),
            (One(Sheet), "cond_formats") => List(CondFormat),
            (One(Sheet), "tables") => List(Table),
            (One(Sheet), "autofilter") => One(AutoFilter),
            (One(Sheet), "charts") => List(Chart),
            (One(Sheet), "images") => List(Image),
            (One(Sheet), "shapes") => List(Shape),
            (One(Sheet), "controls") => List(Control),
            (One(Sheet), "slicers") => List(Slicer),
            (One(Sheet), "pivots") => List(Pivot),
            (One(Sheet), "print") => One(Print),
            (One(Sheet), "protection") => One(Protection),
            (One(Sheet), "edit_ranges") => List(EditRange),
            (One(CellStore), "bands") => One(Bands),
            (One(Band), "rows") => One(StoredRows),
            (One(Cell), "value") => One(Value),
            (One(Cell), "py") => One(Python),
            (One(Value), "v") => One(ValuePayload),
            (One(ValuePayload), "data") => List(Value),
            (One(Python), "output") => One(PythonOutput),
            (One(PythonOutput), "value") => One(Value),
            (One(MetadataRecord), "import_issues") => List(Issue),
            (One(MetadataRecord), "original_path_hint") => One(NativePath),
            (One(Sidecars), "workbook") => One(WorkbookSidecar),
            (One(Sidecars), "sheets") => List(SheetSidecar),
            (One(WorkbookSidecar), "source_dir" | "source_path") => One(NativePath),
            (One(WorkbookSidecar), "macros_unread") => List(General),
            (One(WorkbookSidecar), "cell_picture_groups") => List(Group),
            (One(WorkbookSidecar | SheetSidecar), "preserved") => One(Preserved),
            (One(SheetSidecar), "spill") => One(Spill),
            (One(SheetSidecar), "spill_ranges") => One(SpillRanges),
            (One(Preserved), "package_rels" | "rels") => List(Relationship),
            (One(Preserved), "parts") => List(Part),
            (One(Preserved), "vml") => One(OptionalPart),
            (One(Preserved), "elements") => List(Fragment),
            (One(Preserved), "sheets") => List(KeptSheet),
            (One(Part | OptionalPart), "rels") => List(Relationship),
            (One(Fragment), "xml") => One(Xml),
            (One(NativePath), "unix") => List(U8),
            (One(NativePath), "windows") => List(U16),
            (One(StyleTable), "styles") => List(Style),
            (One(Style), "font") => One(Font),
            (One(Style), "dxf_stated") => One(Dxf),
            (One(Theme), "major_font_scripts" | "minor_font_scripts") => List(ScriptPair),
            (One(Phonetic), "props") => One(PhoneticProps),
            (One(Comment), "replies") => List(ReplyPair),
            (One(Comment), "dates") => List(Metadata),
            (One(Table), "columns") => List(TableColumn),
            (One(Table), "filter") | (One(AutoFilter), "criteria") => List(FilterPair),
            (One(TableColumn), "xml") => One(XmlColumn),
            (One(Filter), "Icon") => One(FilterIcon),
            (One(CondFormat), "rule") => One(CfRule),
            (One(CfRule), "TimePeriod") => One(CfTime),
            (One(CfRule), "IconSet") => One(CfIcon),
            (
                One(CfRule),
                "CellIs" | "Expression" | "ContainsText" | "NotContainsText" | "BeginsWith" | "EndsWith" | "Blanks" | "NoBlanks" | "Errors"
                | "NoErrors" | "Duplicate" | "Unique" | "Top10" | "AboveAverage",
            ) => One(CfStyle),
            (One(CfStyle | CfTime), "style") => One(Style),
            (One(Image), "data") => One(Base64),
            (One(Shape), "format") => One(ShapeFormat),
            (One(Control), "action") => One(Action),
            (One(Pivot), "rows" | "columns" | "values" | "filters") => List(PivotField),
            (One(Pivot), "calculated_fields") => List(PivotCalc),
            (One(Protection | EditRange), "password") => One(Password),
            (One(Password), "Iterated") => One(IteratedPassword),
            (One(Macro), "inputs") => List(NamedInput),
            (One(Macro), "steps") => List(Step),
            (One(Macro), "permissions") => List(Metadata),
            (One(Step), "body" | "then" | "else") => List(Step),
            (One(DocProps), "title" | "subject" | "author" | "company" | "keywords" | "description")
            | (One(DefinedName | Sheet | Font | Control | PivotCalc | NamedInput), "name")
            | (One(Theme), "name" | "major_font" | "minor_font" | "major_font_ea" | "minor_font_ea" | "major_font_cs" | "minor_font_cs")
            | (One(Dxf), "scheme")
            | (One(PhoneticProps), "kind" | "alignment")
            | (One(Table), "name" | "style" | "table_type")
            | (One(TableColumn), "name" | "unique_name")
            | (One(XmlColumn), "data_type")
            | (One(CfTime), "period")
            | (One(CfIcon | FilterIcon), "set")
            | (One(Comment), "author")
            | (One(Hyperlink), "target")
            | (One(Chart), "pivot")
            | (One(Image), "mime")
            | (One(ShapeFormat), "font")
            | (One(Action), "name" | "id")
            | (One(Slicer), "table" | "column")
            | (One(Pivot), "name" | "style")
            | (One(PivotField), "source_col")
            | (One(Print), "paper")
            | (One(EditRange), "title")
            | (One(IteratedPassword), "algorithm")
            | (One(PythonOutput), "typeName")
            | (One(Macro), "name" | "shortcut")
            | (One(Step), "id")
            | (One(MetadataRecord), "title")
            | (One(Relationship), "id" | "rel_type" | "target")
            | (One(Issue), "code" | "feature" | "location" | "description")
            | (One(Part | OptionalPart), "name" | "content_type")
            | (One(Fragment), "slot")
            | (One(KeptSheet), "name" | "rel_id") => One(Metadata),
            _ => Self::GENERAL,
        }
    }

    pub(super) fn element(self, index: usize) -> Self {
        use Kind::*;
        match (self, index) {
            (Self::List(kind), _) => Self::One(kind),
            (Self::One(NamedStylePair), 0) | (Self::One(ReplyPair), 0) | (Self::One(ScriptPair), 0 | 1) => Self::One(Metadata),
            (Self::One(NamedStylePair), 1) => Self::One(Style),
            (Self::One(FilterPair), 1) => Self::One(Filter),
            (Self::One(PicturePair), 1) => Self::One(Image),
            _ => Self::GENERAL,
        }
    }

    pub(super) fn key_kind(self) -> Option<KeyKind> {
        use Kind::*;
        match self {
            Self::One(Bands | StoredRows | StoredCols | Lines) => Some(KeyKind::Decimal),
            Self::One(Comments | Hyperlinks | Spill | SpillRanges) => Some(KeyKind::A1),
            _ => None,
        }
    }
}
