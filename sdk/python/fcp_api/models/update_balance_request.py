from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateBalanceRequest")



@_attrs_define
class UpdateBalanceRequest:
    """ Balance settings update. Every field is optional; the balance amount
    itself is only changed through transactions.

        Attributes:
            currency (None | str | Unset):
            credit_limit (float | None | Unset):
            enforce (bool | None | Unset):
            low_balance_threshold (float | None | Unset): `null` clears the threshold; omit to leave it unchanged.
     """

    currency: None | str | Unset = UNSET
    credit_limit: float | None | Unset = UNSET
    enforce: bool | None | Unset = UNSET
    low_balance_threshold: float | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        currency: None | str | Unset
        if isinstance(self.currency, Unset):
            currency = UNSET
        else:
            currency = self.currency

        credit_limit: float | None | Unset
        if isinstance(self.credit_limit, Unset):
            credit_limit = UNSET
        else:
            credit_limit = self.credit_limit

        enforce: bool | None | Unset
        if isinstance(self.enforce, Unset):
            enforce = UNSET
        else:
            enforce = self.enforce

        low_balance_threshold: float | None | Unset
        if isinstance(self.low_balance_threshold, Unset):
            low_balance_threshold = UNSET
        else:
            low_balance_threshold = self.low_balance_threshold


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if currency is not UNSET:
            field_dict["currency"] = currency
        if credit_limit is not UNSET:
            field_dict["credit_limit"] = credit_limit
        if enforce is not UNSET:
            field_dict["enforce"] = enforce
        if low_balance_threshold is not UNSET:
            field_dict["low_balance_threshold"] = low_balance_threshold

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_currency(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        currency = _parse_currency(d.pop("currency", UNSET))


        def _parse_credit_limit(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        credit_limit = _parse_credit_limit(d.pop("credit_limit", UNSET))


        def _parse_enforce(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enforce = _parse_enforce(d.pop("enforce", UNSET))


        def _parse_low_balance_threshold(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        low_balance_threshold = _parse_low_balance_threshold(d.pop("low_balance_threshold", UNSET))


        update_balance_request = cls(
            currency=currency,
            credit_limit=credit_limit,
            enforce=enforce,
            low_balance_threshold=low_balance_threshold,
        )


        update_balance_request.additional_properties = d
        return update_balance_request

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
