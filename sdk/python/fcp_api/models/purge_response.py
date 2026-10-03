from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.purge_report import PurgeReport





T = TypeVar("T", bound="PurgeResponse")



@_attrs_define
class PurgeResponse:
    """ What a purge did.

        Attributes:
            user_id (str):
            username (str):
            report (PurgeReport): What a purge did, counted.
     """

    user_id: str
    username: str
    report: PurgeReport
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.purge_report import PurgeReport # noqa: PLC0415
        user_id = self.user_id

        username = self.username

        report = self.report.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "user_id": user_id,
            "username": username,
            "report": report,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.purge_report import PurgeReport # noqa: PLC0415
        d = dict(src_dict)
        user_id = d.pop("user_id")

        username = d.pop("username")

        report = PurgeReport.from_dict(d.pop("report"))




        purge_response = cls(
            user_id=user_id,
            username=username,
            report=report,
        )


        purge_response.additional_properties = d
        return purge_response

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
