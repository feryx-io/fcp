from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="BalanceResponse")



@_attrs_define
class BalanceResponse:
    """ Balance response.

        Attributes:
            tenant_id (str):
            balance (float):
            currency (str):
            credit_limit (float):
            available (float): Funds that may still be spent (balance + credit limit).
            enforce (bool):
            updated_at (datetime.datetime):
            low_balance_threshold (float | None | Unset):
     """

    tenant_id: str
    balance: float
    currency: str
    credit_limit: float
    available: float
    enforce: bool
    updated_at: datetime.datetime
    low_balance_threshold: float | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        tenant_id = self.tenant_id

        balance = self.balance

        currency = self.currency

        credit_limit = self.credit_limit

        available = self.available

        enforce = self.enforce

        updated_at = self.updated_at.isoformat()

        low_balance_threshold: float | None | Unset
        if isinstance(self.low_balance_threshold, Unset):
            low_balance_threshold = UNSET
        else:
            low_balance_threshold = self.low_balance_threshold


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "balance": balance,
            "currency": currency,
            "credit_limit": credit_limit,
            "available": available,
            "enforce": enforce,
            "updated_at": updated_at,
        })
        if low_balance_threshold is not UNSET:
            field_dict["low_balance_threshold"] = low_balance_threshold

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        balance = d.pop("balance")

        currency = d.pop("currency")

        credit_limit = d.pop("credit_limit")

        available = d.pop("available")

        enforce = d.pop("enforce")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_low_balance_threshold(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        low_balance_threshold = _parse_low_balance_threshold(d.pop("low_balance_threshold", UNSET))


        balance_response = cls(
            tenant_id=tenant_id,
            balance=balance,
            currency=currency,
            credit_limit=credit_limit,
            available=available,
            enforce=enforce,
            updated_at=updated_at,
            low_balance_threshold=low_balance_threshold,
        )


        balance_response.additional_properties = d
        return balance_response

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
