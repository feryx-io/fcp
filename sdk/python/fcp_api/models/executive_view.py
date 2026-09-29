from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.executive_alert import ExecutiveAlert
from ..models.executive_filter import ExecutiveFilter
from typing import cast

if TYPE_CHECKING:
  from ..models.assistant_view import AssistantView





T = TypeVar("T", bound="ExecutiveView")



@_attrs_define
class ExecutiveView:
    """ A user as an executive, as the API shows it: user ids in their
    prefixed form, as everywhere else.

        Attributes:
            enabled (bool):
            filter_ (ExecutiveFilter): Which of an executive's calls their assistants take first
                (`docs/ENTERPRISE_CALLING.md` §3.10).
            alert (ExecutiveAlert): Who rings for a call the filter takes.
            assistants (list[AssistantView]):
     """

    enabled: bool
    filter_: ExecutiveFilter
    alert: ExecutiveAlert
    assistants: list[AssistantView]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.assistant_view import AssistantView # noqa: PLC0415
        enabled = self.enabled

        filter_ = self.filter_.value

        alert = self.alert.value

        assistants = []
        for assistants_item_data in self.assistants:
            assistants_item = assistants_item_data.to_dict()
            assistants.append(assistants_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "enabled": enabled,
            "filter": filter_,
            "alert": alert,
            "assistants": assistants,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.assistant_view import AssistantView # noqa: PLC0415
        d = dict(src_dict)
        enabled = d.pop("enabled")

        filter_ = ExecutiveFilter(d.pop("filter"))




        alert = ExecutiveAlert(d.pop("alert"))




        assistants = []
        _assistants = d.pop("assistants")
        for assistants_item_data in (_assistants):
            assistants_item = AssistantView.from_dict(assistants_item_data)



            assistants.append(assistants_item)


        executive_view = cls(
            enabled=enabled,
            filter_=filter_,
            alert=alert,
            assistants=assistants,
        )


        executive_view.additional_properties = d
        return executive_view

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
