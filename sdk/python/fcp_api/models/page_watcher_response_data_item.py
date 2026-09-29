from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="PageWatcherResponseDataItem")



@_attrs_define
class PageWatcherResponseDataItem:
    """ Watcher info response.

        Attributes:
            watcher_aor (str):
            subscription_id (str):
            event_package (str):
            expires_in_secs (int):
     """

    watcher_aor: str
    subscription_id: str
    event_package: str
    expires_in_secs: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        watcher_aor = self.watcher_aor

        subscription_id = self.subscription_id

        event_package = self.event_package

        expires_in_secs = self.expires_in_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "watcher_aor": watcher_aor,
            "subscription_id": subscription_id,
            "event_package": event_package,
            "expires_in_secs": expires_in_secs,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        watcher_aor = d.pop("watcher_aor")

        subscription_id = d.pop("subscription_id")

        event_package = d.pop("event_package")

        expires_in_secs = d.pop("expires_in_secs")

        page_watcher_response_data_item = cls(
            watcher_aor=watcher_aor,
            subscription_id=subscription_id,
            event_package=event_package,
            expires_in_secs=expires_in_secs,
        )


        page_watcher_response_data_item.additional_properties = d
        return page_watcher_response_data_item

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
