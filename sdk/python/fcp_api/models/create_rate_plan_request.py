from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="CreateRatePlanRequest")



@_attrs_define
class CreateRatePlanRequest:
    """ Create rate plan request.

        Attributes:
            name (str):
            currency (str | Unset):
            default_rate_per_minute (float | Unset):
            connect_fee (float | Unset):
            billing_increment_secs (int | Unset):
            is_default (bool | Unset):
            enabled (bool | Unset):
     """

    name: str
    currency: str | Unset = UNSET
    default_rate_per_minute: float | Unset = UNSET
    connect_fee: float | Unset = UNSET
    billing_increment_secs: int | Unset = UNSET
    is_default: bool | Unset = UNSET
    enabled: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        currency = self.currency

        default_rate_per_minute = self.default_rate_per_minute

        connect_fee = self.connect_fee

        billing_increment_secs = self.billing_increment_secs

        is_default = self.is_default

        enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
        })
        if currency is not UNSET:
            field_dict["currency"] = currency
        if default_rate_per_minute is not UNSET:
            field_dict["default_rate_per_minute"] = default_rate_per_minute
        if connect_fee is not UNSET:
            field_dict["connect_fee"] = connect_fee
        if billing_increment_secs is not UNSET:
            field_dict["billing_increment_secs"] = billing_increment_secs
        if is_default is not UNSET:
            field_dict["is_default"] = is_default
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        currency = d.pop("currency", UNSET)

        default_rate_per_minute = d.pop("default_rate_per_minute", UNSET)

        connect_fee = d.pop("connect_fee", UNSET)

        billing_increment_secs = d.pop("billing_increment_secs", UNSET)

        is_default = d.pop("is_default", UNSET)

        enabled = d.pop("enabled", UNSET)

        create_rate_plan_request = cls(
            name=name,
            currency=currency,
            default_rate_per_minute=default_rate_per_minute,
            connect_fee=connect_fee,
            billing_increment_secs=billing_increment_secs,
            is_default=is_default,
            enabled=enabled,
        )


        create_rate_plan_request.additional_properties = d
        return create_rate_plan_request

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
