from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.balance_transaction_kind import BalanceTransactionKind
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="BalanceTransactionResponse")



@_attrs_define
class BalanceTransactionResponse:
    """ Ledger entry response.

        Attributes:
            id (str):
            tenant_id (str):
            kind (BalanceTransactionKind): Why a balance changed.
            amount (float):
            balance_after (float):
            currency (str):
            created_at (datetime.datetime):
            call_id (None | str | Unset):
            description (None | str | Unset):
            external_ref (None | str | Unset):
     """

    id: str
    tenant_id: str
    kind: BalanceTransactionKind
    amount: float
    balance_after: float
    currency: str
    created_at: datetime.datetime
    call_id: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    external_ref: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        kind = self.kind.value

        amount = self.amount

        balance_after = self.balance_after

        currency = self.currency

        created_at = self.created_at.isoformat()

        call_id: None | str | Unset
        if isinstance(self.call_id, Unset):
            call_id = UNSET
        else:
            call_id = self.call_id

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        external_ref: None | str | Unset
        if isinstance(self.external_ref, Unset):
            external_ref = UNSET
        else:
            external_ref = self.external_ref


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "kind": kind,
            "amount": amount,
            "balance_after": balance_after,
            "currency": currency,
            "created_at": created_at,
        })
        if call_id is not UNSET:
            field_dict["call_id"] = call_id
        if description is not UNSET:
            field_dict["description"] = description
        if external_ref is not UNSET:
            field_dict["external_ref"] = external_ref

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        kind = BalanceTransactionKind(d.pop("kind"))




        amount = d.pop("amount")

        balance_after = d.pop("balance_after")

        currency = d.pop("currency")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_call_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        call_id = _parse_call_id(d.pop("call_id", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_external_ref(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        external_ref = _parse_external_ref(d.pop("external_ref", UNSET))


        balance_transaction_response = cls(
            id=id,
            tenant_id=tenant_id,
            kind=kind,
            amount=amount,
            balance_after=balance_after,
            currency=currency,
            created_at=created_at,
            call_id=call_id,
            description=description,
            external_ref=external_ref,
        )


        balance_transaction_response.additional_properties = d
        return balance_transaction_response

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
