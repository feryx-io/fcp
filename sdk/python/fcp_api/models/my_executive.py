from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.executive_filter import ExecutiveFilter
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="MyExecutive")



@_attrs_define
class MyExecutive:
    """ An executive the user assists (`docs/ENTERPRISE_CALLING.md` §3.10).

        Attributes:
            user_id (str):
            username (str):
            opted_in (bool): Whether the user takes this executive's calls.
            enabled (bool): Whether the executive has their assistants' handling on.
            filter_ (ExecutiveFilter): Which of an executive's calls their assistants take first
                (`docs/ENTERPRISE_CALLING.md` §3.10).
            display_name (None | str | Unset):
     """

    user_id: str
    username: str
    opted_in: bool
    enabled: bool
    filter_: ExecutiveFilter
    display_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        user_id = self.user_id

        username = self.username

        opted_in = self.opted_in

        enabled = self.enabled

        filter_ = self.filter_.value

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "user_id": user_id,
            "username": username,
            "opted_in": opted_in,
            "enabled": enabled,
            "filter": filter_,
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        user_id = d.pop("user_id")

        username = d.pop("username")

        opted_in = d.pop("opted_in")

        enabled = d.pop("enabled")

        filter_ = ExecutiveFilter(d.pop("filter"))




        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        my_executive = cls(
            user_id=user_id,
            username=username,
            opted_in=opted_in,
            enabled=enabled,
            filter_=filter_,
            display_name=display_name,
        )


        my_executive.additional_properties = d
        return my_executive

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
