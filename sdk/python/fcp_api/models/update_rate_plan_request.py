from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateRatePlanRequest")



@_attrs_define
class UpdateRatePlanRequest:
    """ Update rate plan request (partial).

        Attributes:
            name (None | str | Unset):
            currency (None | str | Unset):
            default_rate_per_minute (float | None | Unset):
            connect_fee (float | None | Unset):
            billing_increment_secs (int | None | Unset):
            is_default (bool | None | Unset):
            enabled (bool | None | Unset):
     """

    name: None | str | Unset = UNSET
    currency: None | str | Unset = UNSET
    default_rate_per_minute: float | None | Unset = UNSET
    connect_fee: float | None | Unset = UNSET
    billing_increment_secs: int | None | Unset = UNSET
    is_default: bool | None | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        currency: None | str | Unset
        if isinstance(self.currency, Unset):
            currency = UNSET
        else:
            currency = self.currency

        default_rate_per_minute: float | None | Unset
        if isinstance(self.default_rate_per_minute, Unset):
            default_rate_per_minute = UNSET
        else:
            default_rate_per_minute = self.default_rate_per_minute

        connect_fee: float | None | Unset
        if isinstance(self.connect_fee, Unset):
            connect_fee = UNSET
        else:
            connect_fee = self.connect_fee

        billing_increment_secs: int | None | Unset
        if isinstance(self.billing_increment_secs, Unset):
            billing_increment_secs = UNSET
        else:
            billing_increment_secs = self.billing_increment_secs

        is_default: bool | None | Unset
        if isinstance(self.is_default, Unset):
            is_default = UNSET
        else:
            is_default = self.is_default

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if name is not UNSET:
            field_dict["name"] = name
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
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_currency(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        currency = _parse_currency(d.pop("currency", UNSET))


        def _parse_default_rate_per_minute(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        default_rate_per_minute = _parse_default_rate_per_minute(d.pop("default_rate_per_minute", UNSET))


        def _parse_connect_fee(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        connect_fee = _parse_connect_fee(d.pop("connect_fee", UNSET))


        def _parse_billing_increment_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        billing_increment_secs = _parse_billing_increment_secs(d.pop("billing_increment_secs", UNSET))


        def _parse_is_default(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        is_default = _parse_is_default(d.pop("is_default", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        update_rate_plan_request = cls(
            name=name,
            currency=currency,
            default_rate_per_minute=default_rate_per_minute,
            connect_fee=connect_fee,
            billing_increment_secs=billing_increment_secs,
            is_default=is_default,
            enabled=enabled,
        )


        update_rate_plan_request.additional_properties = d
        return update_rate_plan_request

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
