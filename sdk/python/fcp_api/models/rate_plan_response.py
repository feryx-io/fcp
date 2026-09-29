from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="RatePlanResponse")



@_attrs_define
class RatePlanResponse:
    """ Rate plan response.

        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            currency (str):
            default_rate_per_minute (float):
            connect_fee (float):
            billing_increment_secs (int):
            is_default (bool):
            enabled (bool):
     """

    id: str
    tenant_id: str
    name: str
    currency: str
    default_rate_per_minute: float
    connect_fee: float
    billing_increment_secs: int
    is_default: bool
    enabled: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

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
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "currency": currency,
            "default_rate_per_minute": default_rate_per_minute,
            "connect_fee": connect_fee,
            "billing_increment_secs": billing_increment_secs,
            "is_default": is_default,
            "enabled": enabled,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        currency = d.pop("currency")

        default_rate_per_minute = d.pop("default_rate_per_minute")

        connect_fee = d.pop("connect_fee")

        billing_increment_secs = d.pop("billing_increment_secs")

        is_default = d.pop("is_default")

        enabled = d.pop("enabled")

        rate_plan_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            currency=currency,
            default_rate_per_minute=default_rate_per_minute,
            connect_fee=connect_fee,
            billing_increment_secs=billing_increment_secs,
            is_default=is_default,
            enabled=enabled,
        )


        rate_plan_response.additional_properties = d
        return rate_plan_response

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
