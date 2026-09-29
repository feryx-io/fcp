from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.export_report_classes import ExportReportClasses
  from ..models.export_report_held_by import ExportReportHeldBy
  from ..models.export_report_not_exported import ExportReportNotExported





T = TypeVar("T", bound="ExportReport")



@_attrs_define
class ExportReport:
    """ What an export found, class by class, and what it could not reach.

        Attributes:
            classes (ExportReportClasses):
            not_exported (ExportReportNotExported): Stores the export does not reach, with why (transit stores, operator
                stores, backups).
            review (list[str] | Unset): Recordings in which other people are heard, for the tenant to review
                before handing the bundle over (`media/<class>/<id>.<ext>`).
            held_by (ExportReportHeldBy | Unset): The holds covering exported items, by id, with their matters.
     """

    classes: ExportReportClasses
    not_exported: ExportReportNotExported
    review: list[str] | Unset = UNSET
    held_by: ExportReportHeldBy | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.export_report_classes import ExportReportClasses # noqa: PLC0415
        from ..models.export_report_held_by import ExportReportHeldBy # noqa: PLC0415
        from ..models.export_report_not_exported import ExportReportNotExported # noqa: PLC0415
        classes = self.classes.to_dict()

        not_exported = self.not_exported.to_dict()

        review: list[str] | Unset = UNSET
        if not isinstance(self.review, Unset):
            review = self.review



        held_by: dict[str, Any] | Unset = UNSET
        if not isinstance(self.held_by, Unset):
            held_by = self.held_by.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "classes": classes,
            "not_exported": not_exported,
        })
        if review is not UNSET:
            field_dict["review"] = review
        if held_by is not UNSET:
            field_dict["held_by"] = held_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.export_report_classes import ExportReportClasses # noqa: PLC0415
        from ..models.export_report_held_by import ExportReportHeldBy # noqa: PLC0415
        from ..models.export_report_not_exported import ExportReportNotExported # noqa: PLC0415
        d = dict(src_dict)
        classes = ExportReportClasses.from_dict(d.pop("classes"))




        not_exported = ExportReportNotExported.from_dict(d.pop("not_exported"))




        review = cast(list[str], d.pop("review", UNSET))


        _held_by = d.pop("held_by", UNSET)
        held_by: ExportReportHeldBy | Unset
        if isinstance(_held_by,  Unset):
            held_by = UNSET
        else:
            held_by = ExportReportHeldBy.from_dict(_held_by)




        export_report = cls(
            classes=classes,
            not_exported=not_exported,
            review=review,
            held_by=held_by,
        )


        export_report.additional_properties = d
        return export_report

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
