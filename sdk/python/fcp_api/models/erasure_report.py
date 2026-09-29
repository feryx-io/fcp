from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.dead_letter_report import DeadLetterReport
  from ..models.erasure_report_classes import ErasureReportClasses
  from ..models.erasure_report_held_by import ErasureReportHeldBy
  from ..models.erasure_report_kept import ErasureReportKept
  from ..models.erasure_report_not_reached import ErasureReportNotReached
  from ..models.erasure_report_tables import ErasureReportTables





T = TypeVar("T", bound="ErasureReport")



@_attrs_define
class ErasureReport:
    """ What an erasure found, did, kept and could not reach.

        Attributes:
            preview (bool): A preview changes nothing.
            classes (ErasureReportClasses):
            kept (ErasureReportKept | Unset): Classes kept, with why.
            held_by (ErasureReportHeldBy | Unset): The holds that kept items, by id, with their matters.
            not_reached (ErasureReportNotReached | Unset): What the erasure cannot reach, and when it goes.
            dead_letters (DeadLetterReport | None | Unset):
            tables (ErasureReportTables | Unset): A tenant's deletion: rows removed, by table.
     """

    preview: bool
    classes: ErasureReportClasses
    kept: ErasureReportKept | Unset = UNSET
    held_by: ErasureReportHeldBy | Unset = UNSET
    not_reached: ErasureReportNotReached | Unset = UNSET
    dead_letters: DeadLetterReport | None | Unset = UNSET
    tables: ErasureReportTables | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.dead_letter_report import DeadLetterReport # noqa: PLC0415
        from ..models.erasure_report_classes import ErasureReportClasses # noqa: PLC0415
        from ..models.erasure_report_held_by import ErasureReportHeldBy # noqa: PLC0415
        from ..models.erasure_report_kept import ErasureReportKept # noqa: PLC0415
        from ..models.erasure_report_not_reached import ErasureReportNotReached # noqa: PLC0415
        from ..models.erasure_report_tables import ErasureReportTables # noqa: PLC0415
        preview = self.preview

        classes = self.classes.to_dict()

        kept: dict[str, Any] | Unset = UNSET
        if not isinstance(self.kept, Unset):
            kept = self.kept.to_dict()

        held_by: dict[str, Any] | Unset = UNSET
        if not isinstance(self.held_by, Unset):
            held_by = self.held_by.to_dict()

        not_reached: dict[str, Any] | Unset = UNSET
        if not isinstance(self.not_reached, Unset):
            not_reached = self.not_reached.to_dict()

        dead_letters: dict[str, Any] | None | Unset
        if isinstance(self.dead_letters, Unset):
            dead_letters = UNSET
        elif isinstance(self.dead_letters, DeadLetterReport):
            dead_letters = self.dead_letters.to_dict()
        else:
            dead_letters = self.dead_letters

        tables: dict[str, Any] | Unset = UNSET
        if not isinstance(self.tables, Unset):
            tables = self.tables.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "preview": preview,
            "classes": classes,
        })
        if kept is not UNSET:
            field_dict["kept"] = kept
        if held_by is not UNSET:
            field_dict["held_by"] = held_by
        if not_reached is not UNSET:
            field_dict["not_reached"] = not_reached
        if dead_letters is not UNSET:
            field_dict["dead_letters"] = dead_letters
        if tables is not UNSET:
            field_dict["tables"] = tables

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.dead_letter_report import DeadLetterReport # noqa: PLC0415
        from ..models.erasure_report_classes import ErasureReportClasses # noqa: PLC0415
        from ..models.erasure_report_held_by import ErasureReportHeldBy # noqa: PLC0415
        from ..models.erasure_report_kept import ErasureReportKept # noqa: PLC0415
        from ..models.erasure_report_not_reached import ErasureReportNotReached # noqa: PLC0415
        from ..models.erasure_report_tables import ErasureReportTables # noqa: PLC0415
        d = dict(src_dict)
        preview = d.pop("preview")

        classes = ErasureReportClasses.from_dict(d.pop("classes"))




        _kept = d.pop("kept", UNSET)
        kept: ErasureReportKept | Unset
        if isinstance(_kept,  Unset):
            kept = UNSET
        else:
            kept = ErasureReportKept.from_dict(_kept)




        _held_by = d.pop("held_by", UNSET)
        held_by: ErasureReportHeldBy | Unset
        if isinstance(_held_by,  Unset):
            held_by = UNSET
        else:
            held_by = ErasureReportHeldBy.from_dict(_held_by)




        _not_reached = d.pop("not_reached", UNSET)
        not_reached: ErasureReportNotReached | Unset
        if isinstance(_not_reached,  Unset):
            not_reached = UNSET
        else:
            not_reached = ErasureReportNotReached.from_dict(_not_reached)




        def _parse_dead_letters(data: object) -> DeadLetterReport | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                dead_letters_type_0 = DeadLetterReport.from_dict(data)



                return dead_letters_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(DeadLetterReport | None | Unset, data)

        dead_letters = _parse_dead_letters(d.pop("dead_letters", UNSET))


        _tables = d.pop("tables", UNSET)
        tables: ErasureReportTables | Unset
        if isinstance(_tables,  Unset):
            tables = UNSET
        else:
            tables = ErasureReportTables.from_dict(_tables)




        erasure_report = cls(
            preview=preview,
            classes=classes,
            kept=kept,
            held_by=held_by,
            not_reached=not_reached,
            dead_letters=dead_letters,
            tables=tables,
        )


        erasure_report.additional_properties = d
        return erasure_report

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
