from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="PageSpeechProviderDataItem")



@_attrs_define
class PageSpeechProviderDataItem:
    """ A provider the tenant may speak with.

        Attributes:
            name (str): What names it in the settings and requests (`account:<id>` for one
                of the tenant's accounts).
            label (str): What to show: the provider's name, or the account's.
            is_default (bool): The TTS server's default: what a tenant without a choice gets.
            account (bool): One of the tenant's own accounts (`docs/TENANT_SPEECH.md` S3).
     """

    name: str
    label: str
    is_default: bool
    account: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        label = self.label

        is_default = self.is_default

        account = self.account


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
            "label": label,
            "is_default": is_default,
            "account": account,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        label = d.pop("label")

        is_default = d.pop("is_default")

        account = d.pop("account")

        page_speech_provider_data_item = cls(
            name=name,
            label=label,
            is_default=is_default,
            account=account,
        )


        page_speech_provider_data_item.additional_properties = d
        return page_speech_provider_data_item

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
