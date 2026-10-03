from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageGroupMemberResponseDataItem")



@_attrs_define
class PageGroupMemberResponseDataItem:
    """ A member as the API answers it.

        Attributes:
            user_id (str):
            username (str):
            account_status (str):
            added_at (datetime.datetime):
            display_name (None | str | Unset):
     """

    user_id: str
    username: str
    account_status: str
    added_at: datetime.datetime
    display_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        user_id = self.user_id

        username = self.username

        account_status = self.account_status

        added_at = self.added_at.isoformat()

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
            "account_status": account_status,
            "added_at": added_at,
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        user_id = d.pop("user_id")

        username = d.pop("username")

        account_status = d.pop("account_status")

        added_at = datetime.datetime.fromisoformat(d.pop("added_at"))




        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        page_group_member_response_data_item = cls(
            user_id=user_id,
            username=username,
            account_status=account_status,
            added_at=added_at,
            display_name=display_name,
        )


        page_group_member_response_data_item.additional_properties = d
        return page_group_member_response_data_item

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
