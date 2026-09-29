from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="PageSubscriptionInfoDataItem")



@_attrs_define
class PageSubscriptionInfoDataItem:
    """ Subscription information response.

        Attributes:
            call_id (str):
            from_tag (str):
            to_tag (str):
            event (str):
            state (str):
            contact (str):
            expires_in_secs (int): The subscription's lifetime, as its `Expires` gave it.
     """

    call_id: str
    from_tag: str
    to_tag: str
    event: str
    state: str
    contact: str
    expires_in_secs: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        from_tag = self.from_tag

        to_tag = self.to_tag

        event = self.event

        state = self.state

        contact = self.contact

        expires_in_secs = self.expires_in_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "from_tag": from_tag,
            "to_tag": to_tag,
            "event": event,
            "state": state,
            "contact": contact,
            "expires_in_secs": expires_in_secs,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        from_tag = d.pop("from_tag")

        to_tag = d.pop("to_tag")

        event = d.pop("event")

        state = d.pop("state")

        contact = d.pop("contact")

        expires_in_secs = d.pop("expires_in_secs")

        page_subscription_info_data_item = cls(
            call_id=call_id,
            from_tag=from_tag,
            to_tag=to_tag,
            event=event,
            state=state,
            contact=contact,
            expires_in_secs=expires_in_secs,
        )


        page_subscription_info_data_item.additional_properties = d
        return page_subscription_info_data_item

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
